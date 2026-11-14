import AVFoundation
import AppKit
import CoreAudio
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
    private let diagnosticTrack: Int?
    private let diagnostics: ((String) -> Void)?
    private var lastRawDiagnostic: UInt64?
    private var lastConvertedDiagnostic: UInt64?

    init(diagnosticTrack: Int? = nil, diagnostics: ((String) -> Void)? = nil) {
        self.diagnosticTrack = diagnostics == nil ? nil : diagnosticTrack
        self.diagnostics = diagnostics
    }

    private func diagnoseRaw(_ input: AVAudioPCMBuffer, timestamp: UInt64) {
        guard let track = diagnosticTrack,
              lastRawDiagnostic.map({ timestamp >= $0 + 5_000_000_000 }) ?? true else { return }
        lastRawDiagnostic = timestamp
        var peak = 0.0
        var nonzero = 0
        var sampleCount = 0
        let buffers = UnsafeMutableAudioBufferListPointer(input.mutableAudioBufferList)
        for buffer in buffers {
            guard let data = buffer.mData else { continue }
            let count = Int(buffer.mDataByteSize) / max(1, Int(input.format.streamDescription.pointee.mBitsPerChannel / 8))
            for index in 0..<count {
                let value: Double
                switch input.format.commonFormat {
                case .pcmFormatFloat32: value = Double(data.assumingMemoryBound(to: Float.self)[index])
                case .pcmFormatFloat64: value = data.assumingMemoryBound(to: Double.self)[index]
                case .pcmFormatInt16: value = Double(data.assumingMemoryBound(to: Int16.self)[index]) / 32768
                case .pcmFormatInt32: value = Double(data.assumingMemoryBound(to: Int32.self)[index]) / 2147483648
                default: continue
                }
                sampleCount += 1
                peak = max(peak, abs(value))
                if value != 0 { nonzero += 1 }
            }
        }
        diagnostics?(String(format: "Meeting PCM raw track=%d pts=%llu frames=%u samples=%d rate=%.0f channels=%u format=%lu interleaved=%d peak=%.6f nonzero=%d", track, timestamp, input.frameLength, sampleCount, input.format.sampleRate, input.format.channelCount, input.format.commonFormat.rawValue, input.format.isInterleaved ? 1 : 0, peak, nonzero))
    }

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
        diagnoseRaw(input, timestamp: timestamp)
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

    func reset() {
        converter = nil
        inputFormat = nil
        nextTimestamp = nil
        nextInputTimestamp = nil
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
                if let track = diagnosticTrack,
                   lastConvertedDiagnostic.map({ timestamp >= $0 + 5_000_000_000 }) ?? true {
                    lastConvertedDiagnostic = timestamp
                    var peak: Float = 0
                    var nonzero = 0
                    for index in 0..<count {
                        peak = max(peak, abs(data[index]))
                        if data[index] != 0 { nonzero += 1 }
                    }
                    diagnostics?(String(format: "Meeting PCM converted track=%d pts=%llu frames=%d rate=16000 channels=1 peak=%.6f nonzero=%d", track, timestamp, count, Double(peak), nonzero))
                }
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

// SCK audio still needs an active display. Keep automatic idle sleep from
// removing it during a capture; explicit lock, sleep and lid closure still win.
private final class CaptureActivity {
    private let token = ProcessInfo.processInfo.beginActivity(
        options: [.idleDisplaySleepDisabled, .idleSystemSleepDisabled],
        reason: "Recording meeting audio")

    deinit {
        ProcessInfo.processInfo.endActivity(token)
    }
}

@available(macOS 15.0, *)
private final class MeetingCapture: NSObject, SCStreamOutput, SCStreamDelegate {
    private let queue = DispatchQueue(label: "handy.meeting.audio")
    // SCK start/stop completions must never wait on the audio callback queue.
    private let control = DispatchQueue(label: "handy.meeting.control")
    private let cancellation = NSLock()
    private var cancelled = false
    private let callback: HandyMeetingCallback
    private let context: UnsafeMutableRawPointer?
    private let microphone: AVCaptureDevice
    private let application: NSRunningApplication?
    private var applicationPID: Int32 = 0
    // Callback state belongs to queue; stream/recovery/listeners belong to control.
    private let tracks: [AudioTrack]
    private let diagnostics: ((String) -> Void)?
    private var activeStream: SCStream?
    private var accepting = true
    private var firstError: String?
    private var monitor: DispatchSourceTimer?
    private var stream: SCStream?
    private var streamHasStopped = false
    private var displayID: CGDirectDisplayID?
    private var activity: CaptureActivity?
    private var generation: UInt64 = 0
    private var recoveryTimer: DispatchSourceTimer?
    private var recoveryReason = ""
    private var recoveryAttempt = 0
    private var lastRecoveryStarted: UInt64?
    private var unstableSourceFailures = 0
    private var displayObserver: NSObjectProtocol?
    private struct AudioListener {
        let object: AudioObjectID
        var address: AudioObjectPropertyAddress
        let block: AudioObjectPropertyListenerBlock
    }
    private var audioListeners: [AudioListener] = []
    private var hardwareState: [Double] = []

    init(microphone: AVCaptureDevice, application: NSRunningApplication?,
         callback: @escaping HandyMeetingCallback, context: UnsafeMutableRawPointer?) {
        self.microphone = microphone
        self.application = application
        self.callback = callback
        self.context = context
        let diagnostics: ((String) -> Void)?
        if Bundle.main.object(forInfoDictionaryKey: "CFBundleName") as? String == "Handy Meetings Dev" {
            diagnostics = { message in
                message.withCString { callback(context, 254, 0, nil, 0, $0) }
            }
        } else {
            diagnostics = nil
        }
        self.diagnostics = diagnostics
        self.tracks = [AudioTrack(diagnosticTrack: 0, diagnostics: diagnostics),
                       AudioTrack(diagnosticTrack: 1, diagnostics: diagnostics)]
    }

    private var isCancelled: Bool {
        cancellation.lock()
        defer { cancellation.unlock() }
        return cancelled
    }

    private static func preferredDisplay(_ displays: [SCDisplay]) -> SCDisplay? {
        let active = displays.filter { CGDisplayIsActive($0.displayID) != 0 }
        return active.first(where: { CGDisplayIsBuiltin($0.displayID) != 0 }) ?? active.first
    }

    func start(content: SCShareableContent, applicationPID: Int32) throws {
        try control.sync {
            self.applicationPID = applicationPID
            activity = CaptureActivity()
            installObservers()
            try openStream(content: content)
        }
        queue.sync {
            let timer = DispatchSource.makeTimerSource(queue: queue)
            timer.schedule(deadline: .now() + 1, repeating: 1)
            timer.setEventHandler { [weak self] in
                guard let self, !self.isCancelled else { return }
                if let error = self.firstError {
                    // Retry terminal errors when Rust's bounded queue was full.
                    error.withCString { self.callback(self.context, 0, 0, nil, 0, $0) }
                } else if !self.microphone.isConnected {
                    self.fail("The selected microphone disconnected. Reconnect it and start a new capture.")
                } else if self.application?.isTerminated == true {
                    self.fail("The selected application exited. Start a new capture after reopening it.")
                }
            }
            monitor = timer
            timer.resume()
        }
    }

    private func openStream(content: SCShareableContent) throws {
        guard !isCancelled else { return }
        guard microphone.isConnected else {
            throw CaptureFailure(message: "The selected microphone disconnected. Reconnect it and start a new capture.")
        }
        guard let display = Self.preferredDisplay(content.displays) else {
            throw CaptureFailure(message: "No active display is available for the system audio filter.")
        }
        let filter: SCContentFilter
        if applicationPID > 0 {
            guard let app = content.applications.first(where: { $0.processID == applicationPID }),
                  application?.isTerminated != true else {
                throw CaptureFailure(message: "The selected application exited. Select a running application and retry.")
            }
            // Keep the exact PID on every restart; never fall back to global audio.
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
        // No screen output or encoder is registered; SCK still requires a display.
        config.width = 2
        config.height = 2
        config.minimumFrameInterval = CMTime(seconds: 1, preferredTimescale: 1)
        config.showsCursor = false
        let stream = SCStream(filter: filter, configuration: config, delegate: self)
        self.stream = stream
        streamHasStopped = false
        displayID = display.displayID
        generation += 1
        diagnostics?(String(format: "Meeting stream generation=%llu display=%u builtin=%d", generation, display.displayID, CGDisplayIsBuiltin(display.displayID)))
        queue.sync { activeStream = stream }
        try stream.addStreamOutput(self, type: .audio, sampleHandlerQueue: queue)
        try stream.addStreamOutput(self, type: .microphone, sampleHandlerQueue: queue)
        let done = DispatchSemaphore(value: 0)
        var failure: Error?
        stream.startCapture { error in failure = error; done.signal() }
        done.wait()
        if let failure {
            throw CaptureFailure(message: "Cannot start meeting capture: \(failure.localizedDescription). Check Screen & System Audio Recording and Microphone permissions in System Settings.")
        }
    }

    private func installObservers() {
        displayObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: nil
        ) { [weak self] _ in
            self?.control.async { [weak self] in
                guard let self, !self.isCancelled, let id = self.displayID else { return }
                if CGDisplayIsActive(id) == 0 {
                    self.scheduleRecovery(reason: "display-anchor-unavailable")
                }
            }
        }
        hardwareState = readHardwareState()
        bindAudioListeners()
    }

    private func audioValue<T>(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector,
                               scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal,
                               initial: T) -> T {
        var address = AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
        var value = initial
        var size = UInt32(MemoryLayout<T>.size)
        _ = withUnsafeMutablePointer(to: &value) {
            AudioObjectGetPropertyData(object, &address, 0, nil, &size, $0)
        }
        return value
    }

    private func selectedMicrophoneID() -> AudioObjectID {
        var address = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyTranslateUIDToDevice,
            mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        var uid = microphone.uniqueID as CFString
        var device = AudioObjectID(0)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        let status = withUnsafePointer(to: &uid) {
            AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address,
                UInt32(MemoryLayout<CFString>.size), $0, &size, &device)
        }
        return status == noErr ? device : 0
    }

    private func channelCounts(_ device: AudioObjectID, scope: AudioObjectPropertyScope) -> [Double] {
        var address = AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyStreamConfiguration,
            mScope: scope, mElement: kAudioObjectPropertyElementMain)
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(device, &address, 0, nil, &size) == noErr,
              size >= MemoryLayout<AudioBufferList>.size, size <= 65_536 else { return [] }
        let storage = UnsafeMutableRawPointer.allocate(byteCount: Int(size), alignment: MemoryLayout<AudioBufferList>.alignment)
        defer { storage.deallocate() }
        guard AudioObjectGetPropertyData(device, &address, 0, nil, &size, storage) == noErr else { return [] }
        let buffers = UnsafeMutableAudioBufferListPointer(storage.assumingMemoryBound(to: AudioBufferList.self))
        return [Double(buffers.count)] + buffers.map { Double($0.mNumberChannels) }
    }

    private func readHardwareState() -> [Double] {
        let output = audioValue(AudioObjectID(kAudioObjectSystemObject), kAudioHardwarePropertyDefaultOutputDevice, initial: AudioObjectID(0))
        let input = selectedMicrophoneID()
        var state: [Double] = []
        for (device, scope) in [(output, kAudioDevicePropertyScopeOutput), (input, kAudioDevicePropertyScopeInput)] {
            state += [Double(device), audioValue(device, kAudioDevicePropertyNominalSampleRate, initial: Double(0)),
                      Double(audioValue(device, kAudioDevicePropertyDeviceIsAlive, initial: UInt32(0)))]
            state += channelCounts(device, scope: scope)
        }
        return state
    }

    private func removeAudioListeners() {
        for var listener in audioListeners {
            AudioObjectRemovePropertyListenerBlock(listener.object, &listener.address, control, listener.block)
        }
        audioListeners.removeAll()
    }

    private func bindAudioListeners() {
        removeAudioListeners()
        func listen(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector,
                    scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) {
            guard object != 0 else { return }
            var address = AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
            let block: AudioObjectPropertyListenerBlock = { [weak self] _, _ in
                guard let self, !self.isCancelled else { return }
                let state = self.readHardwareState()
                self.diagnostics?("Meeting hardware notification selector=\(selector) state_changed=\(state != self.hardwareState)")
                guard state != self.hardwareState else { return }
                self.hardwareState = state
                self.diagnostics?(String(format: "Meeting audio hardware changed state=%@", state.prefix(20).map { String($0) }.joined(separator: ",")))
                self.bindAudioListeners()
                self.scheduleRecovery(reason: "audio-hardware-format-changed")
            }
            if AudioObjectAddPropertyListenerBlock(object, &address, control, block) == noErr {
                audioListeners.append(AudioListener(object: object, address: address, block: block))
            }
        }
        listen(AudioObjectID(kAudioObjectSystemObject), kAudioHardwarePropertyDefaultOutputDevice)
        let output = audioValue(AudioObjectID(kAudioObjectSystemObject), kAudioHardwarePropertyDefaultOutputDevice, initial: AudioObjectID(0))
        for (device, scope) in [(output, kAudioDevicePropertyScopeOutput), (selectedMicrophoneID(), kAudioDevicePropertyScopeInput)] {
            listen(device, kAudioDevicePropertyNominalSampleRate)
            listen(device, kAudioDevicePropertyStreamConfiguration, scope: scope)
            listen(device, kAudioDevicePropertyDeviceIsAlive)
        }
    }

    func refresh() {
        control.async { [weak self] in self?.scheduleRecovery(reason: "explicit-refresh") }
    }

    private func scheduleRecovery(reason: String) {
        guard !isCancelled, queue.sync(execute: { accepting }) else { return }
        recoveryReason = reason
        if recoveryTimer == nil {
            recoveryAttempt = 0
            let timer = DispatchSource.makeTimerSource(queue: control)
            timer.setEventHandler { [weak self] in self?.recover() }
            recoveryTimer = timer
            timer.resume()
        }
        // Reuse one timer, including for hardware notification bursts.
        recoveryTimer?.schedule(deadline: .now() + .milliseconds(750), repeating: .never)
    }

    private func notice(_ message: String) {
        queue.sync {
            guard accepting, !isCancelled else { return }
            message.withCString { callback(context, 255, 0, nil, 0, $0) }
        }
    }

    private func recover() {
        guard !isCancelled, queue.sync(execute: { accepting }) else {
            recoveryTimer?.cancel(); recoveryTimer = nil
            return
        }
        recoveryAttempt += 1
        lastRecoveryStarted = DispatchTime.now().uptimeNanoseconds
        diagnostics?(String(format: "Meeting recovery reason=%@ generation=%llu display=%u attempt=%d", recoveryReason, generation, displayID ?? 0, recoveryAttempt))
        if recoveryAttempt == 1 { notice("Meeting audio capture is restarting after a capture source or audio hardware change.") }
        _ = closeStream(final: false)
        guard !isCancelled, queue.sync(execute: { accepting }) else { return }
        do {
            let content = try shareableContent()
            guard !isCancelled else { return }
            try openStream(content: content)
            guard !isCancelled else { return }
            recoveryTimer?.cancel(); recoveryTimer = nil
            diagnostics?(String(format: "Meeting recovery complete generation=%llu display=%u", generation, displayID ?? 0))
            notice("Meeting audio capture restarted. Any interruption is preserved as a gap in the recording.")
        } catch {
            _ = closeStream(final: false)
            guard !isCancelled else { return }
            if recoveryAttempt < 2, microphone.isConnected, application?.isTerminated != true {
                recoveryTimer?.schedule(deadline: .now() + .seconds(1), repeating: .never)
            } else {
                recoveryTimer?.cancel(); recoveryTimer = nil
                queue.sync { fail("Cannot recover meeting audio capture: \(error.localizedDescription)") }
            }
        }
    }

    private func emit(track: Int, timestamp: UInt64, samples: UnsafePointer<Float>, count: Int) {
        callback(context, UInt8(track), timestamp, samples, count, nil)
    }

    private func fail(_ message: String) {
        guard accepting, firstError == nil else { return }
        firstError = message
        accepting = false
        message.withCString { callback(context, 0, 0, nil, 0, $0) }
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard accepting, activeStream === stream else { return }
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
        control.async { [weak self] in
            guard let self, !self.isCancelled, self.stream === stream else { return }
            self.streamHasStopped = true
            if (error as NSError).code == -3815 {
                let now = DispatchTime.now().uptimeNanoseconds
                if let last = self.lastRecoveryStarted, now - last < 15_000_000_000 {
                    self.unstableSourceFailures += 1
                } else {
                    self.unstableSourceFailures = 0
                }
                if self.unstableSourceFailures >= 2 {
                    self.recoveryTimer?.cancel(); self.recoveryTimer = nil
                    self.queue.sync { self.fail("Meeting audio capture sources remain unavailable after recovery. Start a new capture when a display is available.") }
                    return
                }
                self.scheduleRecovery(reason: "capture-source-unavailable-3815")
            } else {
                self.queue.async { [weak self] in
                    guard let self, self.activeStream === stream else { return }
                    self.fail("Meeting capture stopped: \(error.localizedDescription)")
                }
            }
        }
    }

    // Called only on control. Drain old callbacks/tails before allowing a fresh
    // converter to re-anchor to the new stream's host PTS, preserving real gaps.
    private func closeStream(final: Bool) -> String? {
        var stopFailure: Error?
        if let stream {
            if !streamHasStopped {
                let done = DispatchSemaphore(value: 0)
                stream.stopCapture { error in stopFailure = error; done.signal() }
                done.wait()
            }
            try? stream.removeStreamOutput(self, type: .audio)
            try? stream.removeStreamOutput(self, type: .microphone)
        }
        stream = nil
        return queue.sync {
            activeStream = nil
            if accepting {
                for track in 0..<tracks.count {
                    do {
                        try tracks[track].finish { timestamp, samples, count in
                            self.emit(track: track, timestamp: timestamp, samples: samples, count: count)
                        }
                    } catch { fail(error.localizedDescription) }
                }
            }
            if final { accepting = false }
            else { tracks.forEach { $0.reset() } }
            return firstError ?? stopFailure.map { "Cannot finalize meeting capture: \($0.localizedDescription)" }
        }
    }

    // Cancellation is visible even while control waits for SCK. On return, no
    // observer, delayed recovery, delegate or converter can call Rust's context.
    func stop() -> String? {
        cancellation.lock()
        cancelled = true
        cancellation.unlock()
        return control.sync {
            defer { activity = nil }
            recoveryTimer?.cancel(); recoveryTimer = nil
            if let displayObserver { NotificationCenter.default.removeObserver(displayObserver) }
            displayObserver = nil
            removeAudioListeners()
            queue.sync { monitor?.cancel(); monitor = nil }
            return closeStream(final: true)
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

// Borrow the live handle; the caller serializes refresh against stop.
@_cdecl("handy_meeting_refresh")
func meetingRefresh(_ handle: UnsafeMutableRawPointer?) {
    guard #available(macOS 15.0, *), let handle else { return }
    Unmanaged<MeetingCapture>.fromOpaque(handle).takeUnretainedValue().refresh()
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
