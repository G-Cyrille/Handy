import AVFoundation
import AppKit
import CoreMedia
import ScreenCaptureKit

private let captureLock = NSLock()
private var captureReserved = false
private let unsupported = "Meeting capture requires macOS 15 or later."
private let screenPermission = "Allow Handy in System Settings > Privacy & Security > Screen & System Audio Recording, then restart Handy and retry."
private let microphonePermission = "Allow Handy in System Settings > Privacy & Security > Microphone, then retry."

private struct CaptureFailure: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

private func writeError(_ error: Error, _ destination: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) {
    destination?.pointee = strdup(error.localizedDescription)
}

private func requireWorker() throws {
    if Thread.isMainThread {
        throw CaptureFailure(message: "Run meeting capture operations on a worker thread.")
    }
}

private func requireScreenPermission() throws {
    if !CGPreflightScreenCaptureAccess() && !CGRequestScreenCaptureAccess() {
        throw CaptureFailure(message: screenPermission)
    }
}

@available(macOS 15.0, *)
private func microphones() -> [AVCaptureDevice] {
    AVCaptureDevice.DiscoverySession(deviceTypes: [.microphone, .external],
        mediaType: .audio, position: .unspecified).devices
}

@available(macOS 15.0, *)
private func shareableContent() throws -> SCShareableContent {
    let done = DispatchSemaphore(value: 0)
    var result: SCShareableContent?
    var failure: Error?
    SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: false) { content, error in
        result = content
        failure = error
        done.signal()
    }
    done.wait()
    if let failure {
        throw CaptureFailure(message: "Cannot list capture applications: \(failure.localizedDescription). \(screenPermission)")
    }
    guard let result else { throw CaptureFailure(message: "ScreenCaptureKit returned no capture sources.") }
    return result
}

@available(macOS 15.0, *)
private func requireMicrophonePermission() throws {
    switch AVCaptureDevice.authorizationStatus(for: .audio) {
    case .authorized: return
    case .notDetermined:
        guard Bundle.main.object(forInfoDictionaryKey: "NSMicrophoneUsageDescription") is String else {
            throw CaptureFailure(message: "The app bundle is missing NSMicrophoneUsageDescription. Launch the signed Handy app with microphone usage configured.")
        }
        let done = DispatchSemaphore(value: 0)
        var allowed = false
        AVCaptureDevice.requestAccess(for: .audio) { granted in
            allowed = granted
            done.signal()
        }
        done.wait()
        if !allowed { throw CaptureFailure(message: microphonePermission) }
    default: throw CaptureFailure(message: microphonePermission)
    }
}

// One converter per track preserves resampling state across sample buffers.
@available(macOS 15.0, *)
private final class AudioTrack {
    private var converter: AVAudioConverter?
    private var inputFormat: AVAudioFormat?
    private let outputFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32,
        sampleRate: 16_000, channels: 1, interleaved: false)!
    private var nextTimestamp: UInt64?
    private var nextInputTimestamp: UInt64?

    func convert(_ sample: CMSampleBuffer, emit: (UInt64, UnsafePointer<Float>, Int) -> Void) throws {
        let count = CMSampleBufferGetNumSamples(sample)
        guard count > 0, count <= 65_536,
              let description = CMSampleBufferGetFormatDescription(sample),
              let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(description),
              asbd.pointee.mFormatID == kAudioFormatLinearPCM else {
            throw CaptureFailure(message: "Capture returned an unsupported or oversized PCM audio buffer.")
        }
        let format = AVAudioFormat(cmAudioFormatDescription: description)
        guard format.sampleRate >= 8_000, format.sampleRate <= 192_000,
              format.channelCount > 0, format.channelCount <= 32,
              let input = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(count)) else {
            throw CaptureFailure(message: "Capture returned an unsupported or oversized PCM audio buffer.")
        }
        let pts = CMSampleBufferGetPresentationTimeStamp(sample)
        guard pts.isNumeric, pts >= .zero else {
            throw CaptureFailure(message: "Capture returned an invalid host-clock audio timestamp.")
        }
        let timestamp = UInt64(CMTimeConvertScale(pts, timescale: 1_000_000_000, method: .default).value)
        input.frameLength = AVAudioFrameCount(count)
        let status = CMSampleBufferCopyPCMDataIntoAudioBufferList(sample, at: 0,
            frameCount: Int32(count), into: input.mutableAudioBufferList)
        guard status == noErr else {
            throw CaptureFailure(message: "Cannot copy captured PCM audio (OSStatus \(status)).")
        }
        if let expected = nextInputTimestamp, timestamp + 2_000_000 < expected {
            throw CaptureFailure(message: "Capture audio timestamps moved backwards on the host clock.")
        }
        let gap = nextInputTimestamp.map { timestamp > $0 + 2_000_000 } ?? false
        // Device clocks may drift from the host clock over a long meeting.
        // Re-anchor output PTS before that drift can separate the two tracks.
        let drift = nextTimestamp.map { abs(Int64(timestamp) - Int64($0)) } ?? 0
        if inputFormat != format || gap || drift > 20_000_000 {
            if converter != nil { try finish(emit: emit) }
            inputFormat = format
            converter = AVAudioConverter(from: format, to: outputFormat)
            // Normal priming preserves the first input sample's PTS. Converter
            // lookahead is drained at stop, rather than treated as an input gap.
            converter?.primeMethod = .normal
            converter?.downmix = true
            nextTimestamp = timestamp
        }
        guard let converter else { throw CaptureFailure(message: "Cannot create the 16 kHz audio converter.") }
        nextInputTimestamp = timestamp + UInt64((Double(count) / format.sampleRate * 1_000_000_000).rounded())
        var supplied = false
        try drain(converter, capacity: min(131_136, Int(ceil(Double(count) * 16_000 / format.sampleRate)) + 64), emit: emit) { _, state in
            if supplied { state.pointee = .noDataNow; return nil }
            supplied = true
            state.pointee = .haveData
            return input
        }
    }

    func finish(emit: (UInt64, UnsafePointer<Float>, Int) -> Void) throws {
        guard let converter else { return }
        try drain(converter, capacity: 4096, emit: emit) { _, state in
            state.pointee = .endOfStream
            return nil
        }
    }

    private func drain(_ converter: AVAudioConverter, capacity: Int,
        emit: (UInt64, UnsafePointer<Float>, Int) -> Void,
        input: @escaping AVAudioConverterInputBlock) throws {
        guard let output = AVAudioPCMBuffer(pcmFormat: outputFormat, frameCapacity: AVAudioFrameCount(capacity)) else {
            throw CaptureFailure(message: "Cannot allocate the audio conversion buffer.")
        }
        // The input size is bounded. This also bounds work if a converter misbehaves.
        for _ in 0..<64 {
            var failure: NSError?
            let status = converter.convert(to: output, error: &failure, withInputFrom: input)
            if status == .error {
                throw CaptureFailure(message: "Audio conversion failed: \(failure?.localizedDescription ?? "unknown converter error")")
            }
            let count = Int(output.frameLength)
            if count > 0, let data = output.floatChannelData?[0], let timestamp = nextTimestamp {
                for offset in stride(from: 0, to: count, by: 4096) {
                    emit(timestamp + UInt64(offset) * 62_500, data.advanced(by: offset), min(4096, count - offset))
                }
                nextTimestamp = timestamp + UInt64(count) * 62_500
            }
            if status == .inputRanDry || status == .endOfStream { return }
        }
        throw CaptureFailure(message: "Audio converter exceeded its bounded output limit.")
    }
}

@available(macOS 15.0, *)
private final class MeetingCapture: NSObject, SCStreamOutput, SCStreamDelegate {
    private let queue = DispatchQueue(label: "handy.meeting.audio")
    private let callback: HandyMeetingCallback
    private let context: UnsafeMutableRawPointer?
    private let microphone: AVCaptureDevice
    private let application: NSRunningApplication?
    private let tracks = [AudioTrack(), AudioTrack()]
    private var stream: SCStream?
    private var monitor: DispatchSourceTimer?
    // All mutable callback state is confined to queue.
    private var accepting = true
    private var firstError: String?
    private var stopping = false

    init(microphone: AVCaptureDevice, application: NSRunningApplication?,
         callback: @escaping HandyMeetingCallback, context: UnsafeMutableRawPointer?) {
        self.microphone = microphone
        self.application = application
        self.callback = callback
        self.context = context
    }

    func start(content: SCShareableContent, applicationPID: Int32) throws {
        guard let display = content.displays.first else { throw CaptureFailure(message: "No display is available for the system audio filter.") }
        let filter: SCContentFilter
        if applicationPID > 0 {
            guard let app = content.applications.first(where: { $0.processID == applicationPID }) else {
                throw CaptureFailure(message: "The selected application exited. Select a running application and retry.")
            }
            // Application audio covers all its windows/tabs; there is no tab filter.
            filter = SCContentFilter(display: display, including: [app], exceptingWindows: [])
        } else {
            filter = SCContentFilter(display: display, excludingApplications: [], exceptingWindows: [])
        }
        let config = SCStreamConfiguration()
        config.capturesAudio = true
        config.captureMicrophone = true
        config.microphoneCaptureDeviceID = microphone.uniqueID
        config.sampleRate = 48_000
        config.channelCount = 2
        config.excludesCurrentProcessAudio = false
        // ScreenCaptureKit needs a display filter, but no screen output or encoder
        // is registered. Keep its unused display configuration minimal.
        config.width = 2
        config.height = 2
        config.minimumFrameInterval = CMTime(seconds: 1, preferredTimescale: 1)
        config.showsCursor = false
        let stream = SCStream(filter: filter, configuration: config, delegate: self)
        self.stream = stream
        try stream.addStreamOutput(self, type: .audio, sampleHandlerQueue: queue)
        try stream.addStreamOutput(self, type: .microphone, sampleHandlerQueue: queue)
        let done = DispatchSemaphore(value: 0)
        var failure: Error?
        stream.startCapture { error in failure = error; done.signal() }
        done.wait()
        if let failure {
            throw CaptureFailure(message: "Cannot start meeting capture: \(failure.localizedDescription). Check Screen & System Audio Recording and Microphone permissions in System Settings.")
        }
        queue.sync {
            let timer = DispatchSource.makeTimerSource(queue: queue)
            timer.schedule(deadline: .now() + 1, repeating: 1)
            timer.setEventHandler { [weak self] in
                guard let self, !self.stopping else { return }
                if let error = self.firstError {
                    // Retry terminal errors when Rust's bounded queue was full.
                    error.withCString { self.callback(self.context, 0, 0, nil, 0, $0) }
                    return
                }
                if !self.microphone.isConnected {
                    self.fail("The selected microphone disconnected. Reconnect it and start a new capture.")
                } else if self.application?.isTerminated == true {
                    self.fail("The selected application exited. Start a new capture after reopening it.")
                }
            }
            monitor = timer
            timer.resume()
        }
    }

    private func emit(track: Int, timestamp: UInt64, samples: UnsafePointer<Float>, count: Int) {
        callback(context, UInt8(track), timestamp, samples, count, nil)
    }

    private func fail(_ message: String) {
        guard accepting, firstError == nil else { return }
        firstError = message
        // A partial recording must be marked failed, never silently continued.
        accepting = false
        message.withCString { callback(context, 0, 0, nil, 0, $0) }
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard accepting else { return }
        guard CMSampleBufferDataIsReady(sampleBuffer) else {
            fail("Capture returned audio that is not ready.")
            return
        }
        let track: Int
        switch type {
        case .microphone: track = 0
        case .audio: track = 1
        default: return
        }
        do {
            try tracks[track].convert(sampleBuffer) { timestamp, samples, count in
                self.emit(track: track, timestamp: timestamp, samples: samples, count: count)
            }
        } catch { fail(error.localizedDescription) }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        queue.async { [self] in
            if !stopping { fail("Meeting capture stopped: \(error.localizedDescription)") }
        }
    }

    // Wait for SCK completion, drain queued audio and converter tails, then close
    // the callback gate before Rust releases its context. Late delegates see it closed.
    func stop() -> String? {
        queue.sync { stopping = true; monitor?.cancel(); monitor = nil }
        var stopFailure: Error?
        if let stream {
            let done = DispatchSemaphore(value: 0)
            stream.stopCapture { error in stopFailure = error; done.signal() }
            done.wait()
            try? stream.removeStreamOutput(self, type: .audio)
            try? stream.removeStreamOutput(self, type: .microphone)
        }
        return queue.sync {
            if accepting {
                for track in 0..<tracks.count {
                    do {
                        try tracks[track].finish { timestamp, samples, count in
                            self.emit(track: track, timestamp: timestamp, samples: samples, count: count)
                        }
                    } catch { fail(error.localizedDescription) }
                }
            }
            accepting = false
            stream = nil
            return firstError ?? stopFailure.map { "Cannot finalize meeting capture: \($0.localizedDescription)" }
        }
    }
}

@_cdecl("handy_meeting_sources")
func meetingSources(_ result: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?,
                    _ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> Int32 {
    result?.pointee = nil
    errorOut?.pointee = nil
    guard #available(macOS 15.0, *) else {
        result?.pointee = strdup("{\"supported\":false,\"microphones\":[],\"applications\":[]}")
        return 0
    }
    do {
        try requireWorker()
        try requireScreenPermission()
        let content = try shareableContent()
        var seen = Set<Int32>()
        let apps: [[String: Any]] = content.applications.filter {
            $0.processID > 0 && seen.insert($0.processID).inserted
        }.sorted { $0.applicationName.localizedStandardCompare($1.applicationName) == .orderedAscending }
            .map { ["pid": $0.processID, "name": $0.applicationName] }
        let devices = microphones().map { ["id": $0.uniqueID, "name": $0.localizedName] }
        let json = try JSONSerialization.data(withJSONObject: ["supported": true, "microphones": devices, "applications": apps])
        result?.pointee = strdup(String(decoding: json, as: UTF8.self))
        return 0
    } catch { writeError(error, errorOut); return -1 }
}

@_cdecl("handy_meeting_start")
func meetingStart(_ microphoneID: UnsafePointer<CChar>?, _ applicationPID: Int32,
                  _ callback: HandyMeetingCallback?, _ context: UnsafeMutableRawPointer?,
                  _ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> UnsafeMutableRawPointer? {
    errorOut?.pointee = nil
    guard #available(macOS 15.0, *) else {
        writeError(CaptureFailure(message: unsupported), errorOut)
        return nil
    }
    var reserved = false
    var capture: MeetingCapture?
    do {
        try requireWorker()
        captureLock.lock()
        if captureReserved {
            captureLock.unlock()
            throw CaptureFailure(message: "A meeting capture is already running in this process.")
        }
        captureReserved = true
        reserved = true
        captureLock.unlock()
        guard let microphoneID, let callback, applicationPID >= 0 else {
            throw CaptureFailure(message: "Select a microphone and a valid application PID.")
        }
        let id = String(cString: microphoneID)
        guard let microphone = microphones().first(where: { $0.uniqueID == id && $0.isConnected }) else {
            throw CaptureFailure(message: "The selected microphone is unavailable. Select a connected microphone and retry.")
        }
        try requireScreenPermission()
        try requireMicrophonePermission()
        let content = try shareableContent()
        let application = applicationPID > 0 ? NSRunningApplication(processIdentifier: applicationPID) : nil
        if applicationPID > 0 && application == nil {
            throw CaptureFailure(message: "The selected application exited. Select a running application and retry.")
        }
        let session = MeetingCapture(microphone: microphone, application: application,
            callback: callback, context: context)
        capture = session
        try session.start(content: content, applicationPID: applicationPID)
        return Unmanaged.passRetained(session).toOpaque()
    } catch {
        // Even a failed start can have submitted callbacks. Close the gate and
        // wait before returning so the caller can safely release context.
        _ = capture?.stop()
        if reserved {
            captureLock.lock()
            captureReserved = false
            captureLock.unlock()
        }
        writeError(error, errorOut)
        return nil
    }
}

@_cdecl("handy_meeting_stop")
func meetingStop(_ handle: UnsafeMutableRawPointer?,
                 _ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> Int32 {
    errorOut?.pointee = nil
    guard #available(macOS 15.0, *), let handle else { return 0 }
    let capture = Unmanaged<MeetingCapture>.fromOpaque(handle).takeRetainedValue()
    let failure = capture.stop()
    captureLock.lock()
    captureReserved = false
    captureLock.unlock()
    if let failure {
        writeError(CaptureFailure(message: failure), errorOut)
        return -1
    }
    return 0
}

@_cdecl("handy_meeting_host_time_ns")
func meetingHostTimeNS() -> UInt64 {
    let time = CMClockGetTime(CMClockGetHostTimeClock())
    return UInt64(max(0, CMTimeConvertScale(time, timescale: 1_000_000_000, method: .default).value))
}

@_cdecl("handy_meeting_free")
func meetingFree(_ string: UnsafeMutablePointer<CChar>?) { free(string) }
