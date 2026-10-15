@main
private struct AudioConverterTests {
    @available(macOS 15.0, *)
    static func run(rate: Double, channels: Int, interleaved: Bool, drift: Int64 = 0, restartAt: Int? = nil) throws {
        let track = AudioTrack()
        let chunks = drift == 0 ? 100 : 10_000
        let count = Int(rate / 100)
        let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: rate,
            channels: AVAudioChannelCount(channels), interleaved: interleaved)!
        var outputCount = 0
        var inputPTS: UInt64 = 0
        var previousEnd: UInt64?
        var maxSkew: Int64 = 0
        var peak: Float = 0
        var expectRestartAnchor = false
        let emit: (UInt64, UnsafePointer<Float>, Int) -> Void = { timestamp, data, count in
            precondition(count > 0 && count <= 4096)
            if expectRestartAnchor {
                precondition(timestamp == inputPTS, "Restart did not preserve the new stream's host PTS")
                expectRestartAnchor = false
            }
            if drift == 0, let previousEnd {
                precondition(timestamp == previousEnd, "Unexpected PTS discontinuity")
            }
            maxSkew = max(maxSkew, abs(Int64(timestamp) - Int64(inputPTS)))
            previousEnd = timestamp + UInt64(count) * 62_500
            outputCount += count
            for index in 0..<count { peak = max(peak, abs(data[index])) }
        }
        for chunk in 0..<chunks {
            if chunk == restartAt {
                try track.finish(emit: emit)
                track.reset()
                previousEnd = nil
                expectRestartAnchor = true
            }
            let gap: Int64 = restartAt.map { chunk >= $0 ? 2_000_000_000 : 0 } ?? 0
            inputPTS = UInt64(10_000_000_000 + Int64(chunk) * (10_000_000 + drift) + gap)
            let pcm = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(count))!
            pcm.frameLength = AVAudioFrameCount(count)
            for channel in 0..<channels {
                for index in 0..<count {
                    if interleaved {
                        pcm.floatChannelData![0][index * channels + channel] = channel == channels - 1 ? 0.8 : 0
                    } else {
                        pcm.floatChannelData![channel][index] = channel == channels - 1 ? 0.8 : 0
                    }
                }
            }
            var sample: CMSampleBuffer?
            var timing = CMSampleTimingInfo(duration: CMTime(value: 1, timescale: Int32(rate)),
                presentationTimeStamp: CMTime(value: Int64(inputPTS), timescale: 1_000_000_000),
                decodeTimeStamp: .invalid)
            precondition(CMSampleBufferCreate(allocator: nil, dataBuffer: nil, dataReady: false,
                makeDataReadyCallback: nil, refcon: nil, formatDescription: format.formatDescription,
                sampleCount: count, sampleTimingEntryCount: 1, sampleTimingArray: &timing,
                sampleSizeEntryCount: 0, sampleSizeArray: nil, sampleBufferOut: &sample) == noErr)
            precondition(CMSampleBufferSetDataBufferFromAudioBufferList(sample!, blockBufferAllocator: nil,
                blockBufferMemoryAllocator: nil, flags: 0, bufferList: pcm.audioBufferList) == noErr)
            precondition(CMSampleBufferSetDataReady(sample!) == noErr)
            try track.convert(sample!, emit: emit)
        }
        try track.finish(emit: emit)
        precondition(outputCount == chunks * 160, "Resampling changed sample count: \(outputCount)")
        precondition(!expectRestartAnchor, "Restart emitted no audio")
        precondition(peak > 0.1, "Downmix lost the right channel")
        precondition(maxSkew < 35_000_000, "Output lost host-clock alignment: \(maxSkew) ns")
        print("PASS rate=\(rate) channels=\(channels) interleaved=\(interleaved) drift=\(drift)ns/callback restart=\(restartAt != nil) samples=\(outputCount) maxSkew=\(maxSkew)ns")
    }

    static func main() throws {
        guard #available(macOS 15.0, *) else { return }
        for interleaved in [true, false] {
            for (rate, channels) in [(48_000.0, 2), (44_100.0, 1), (16_000.0, 1)] {
                try run(rate: rate, channels: channels, interleaved: interleaved)
            }
        }
        try run(rate: 48_000, channels: 2, interleaved: false, restartAt: 50)
        for drift: Int64 in [-10_000, 10_000] {
            try run(rate: 48_000, channels: 2, interleaved: false, drift: drift)
        }
    }
}
