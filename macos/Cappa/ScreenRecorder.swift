import AVFoundation
import ScreenCaptureKit

enum RecorderError: LocalizedError {
    case noDisplay
    case cannotAddInput
    case noFramesCaptured

    var errorDescription: String? {
        switch self {
        case .noDisplay: return "No display found to record."
        case .cannotAddInput: return "The video writer could not be configured."
        case .noFramesCaptured: return "No video frames were captured."
        }
    }
}

/// Captures one display with ScreenCaptureKit and writes it to an MP4 file with AVAssetWriter.
///
/// All sample handling and pause bookkeeping happens on `queue`; the stream and writer are only
/// set up in `start` before capture begins and torn down in `stop` after it ends. That discipline
/// is what makes it safe to share across threads, hence `@unchecked Sendable`.
final class ScreenRecorder: NSObject, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
    var onStreamError: ((Error) -> Void)?

    private let queue = DispatchQueue(label: "dev.fallberg.cappa.capture")
    private var stream: SCStream?
    private var writer: AVAssetWriter?
    private var videoInput: AVAssetWriterInput?
    private var audioInput: AVAssetWriterInput?
    private var microphoneInput: AVAssetWriterInput?

    private var sessionStarted = false
    // Sample timestamps are on the host clock, so pause gaps are measured on the same clock
    // and subtracted from every later sample.
    private var pausedAt: CMTime?
    private var pauseOffset = CMTime.zero

    /// Microphone capture uses ScreenCaptureKit, which supports it from macOS 15.
    static var supportsMicrophone: Bool {
        if #available(macOS 15.0, *) { return true }
        return false
    }

    /// - Parameters:
    ///   - sourceRect: Display-local area in points (top-left origin) to record, or `nil` for the whole display.
    ///   - captureMicrophone: Records a microphone to its own audio track (macOS 15+).
    ///   - microphoneID: `AVCaptureDevice.uniqueID` of the microphone, or `nil` for the system default input.
    func start(
        display: SCDisplay,
        sourceRect: CGRect?,
        excludingApplications apps: [SCRunningApplication],
        scale: CGFloat,
        captureAudio: Bool,
        captureMicrophone: Bool,
        microphoneID: String?,
        outputURL: URL
    ) async throws {
        let captureSize = sourceRect?.size ?? CGSize(width: display.width, height: display.height)
        // Encoders want even dimensions.
        let width = max(Int(captureSize.width * scale) & ~1, 2)
        let height = max(Int(captureSize.height * scale) & ~1, 2)

        let filter = SCContentFilter(display: display, excludingApplications: apps, exceptingWindows: [])

        let config = SCStreamConfiguration()
        if let sourceRect {
            config.sourceRect = sourceRect
        }
        config.width = width
        config.height = height
        config.minimumFrameInterval = CMTime(value: 1, timescale: 60)
        config.pixelFormat = kCVPixelFormatType_32BGRA
        config.showsCursor = true
        config.queueDepth = 6
        if captureAudio {
            config.capturesAudio = true
            config.excludesCurrentProcessAudio = true
            config.sampleRate = 48_000
            config.channelCount = 2
        }
        let captureMicrophone = captureMicrophone && Self.supportsMicrophone
        if captureMicrophone, #available(macOS 15.0, *) {
            config.captureMicrophone = true
            config.microphoneCaptureDeviceID = microphoneID // nil = the system default input
        }

        let writer = try AVAssetWriter(outputURL: outputURL, fileType: .mp4)

        // H.264 tops out at 4096 px wide in hardware; use HEVC for larger displays.
        let codec: AVVideoCodecType = width > 4096 ? .hevc : .h264
        let videoInput = AVAssetWriterInput(mediaType: .video, outputSettings: [
            AVVideoCodecKey: codec,
            AVVideoWidthKey: width,
            AVVideoHeightKey: height,
            AVVideoCompressionPropertiesKey: [
                AVVideoAverageBitRateKey: width * height * 4,
                AVVideoExpectedSourceFrameRateKey: 60,
            ],
        ])
        videoInput.expectsMediaDataInRealTime = true
        guard writer.canAdd(videoInput) else { throw RecorderError.cannotAddInput }
        writer.add(videoInput)

        var audioInput: AVAssetWriterInput?
        if captureAudio {
            let input = AVAssetWriterInput(mediaType: .audio, outputSettings: [
                AVFormatIDKey: kAudioFormatMPEG4AAC,
                AVSampleRateKey: 48_000,
                AVNumberOfChannelsKey: 2,
                AVEncoderBitRateKey: 160_000,
            ])
            input.expectsMediaDataInRealTime = true
            guard writer.canAdd(input) else { throw RecorderError.cannotAddInput }
            writer.add(input)
            audioInput = input
        }

        // The microphone gets its own track, so system audio and voice stay separate in the recording.
        var microphoneInput: AVAssetWriterInput?
        if captureMicrophone {
            let input = AVAssetWriterInput(mediaType: .audio, outputSettings: [
                AVFormatIDKey: kAudioFormatMPEG4AAC,
                AVSampleRateKey: 48_000,
                AVNumberOfChannelsKey: 1,
                AVEncoderBitRateKey: 96_000,
            ])
            input.expectsMediaDataInRealTime = true
            guard writer.canAdd(input) else { throw RecorderError.cannotAddInput }
            writer.add(input)
            microphoneInput = input
        }

        guard writer.startWriting() else {
            throw writer.error ?? RecorderError.cannotAddInput
        }

        self.writer = writer
        self.videoInput = videoInput
        self.audioInput = audioInput
        self.microphoneInput = microphoneInput

        let stream = SCStream(filter: filter, configuration: config, delegate: self)
        try stream.addStreamOutput(self, type: .screen, sampleHandlerQueue: queue)
        if captureAudio {
            try stream.addStreamOutput(self, type: .audio, sampleHandlerQueue: queue)
        }
        if captureMicrophone, #available(macOS 15.0, *) {
            try stream.addStreamOutput(self, type: .microphone, sampleHandlerQueue: queue)
        }
        try await stream.startCapture()
        self.stream = stream
    }

    func pause() {
        queue.async {
            guard self.pausedAt == nil else { return }
            self.pausedAt = CMClockGetTime(CMClockGetHostTimeClock())
        }
    }

    func resume() {
        queue.async {
            guard let pausedAt = self.pausedAt else { return }
            let now = CMClockGetTime(CMClockGetHostTimeClock())
            self.pauseOffset = CMTimeAdd(self.pauseOffset, CMTimeSubtract(now, pausedAt))
            self.pausedAt = nil
        }
    }

    func stop() async throws {
        if let stream {
            try? await stream.stopCapture()
        }
        stream = nil

        // Let any in-flight sample callbacks finish before closing the file.
        let started: Bool = await withCheckedContinuation { continuation in
            queue.async { continuation.resume(returning: self.sessionStarted) }
        }

        guard let writer else { return }
        guard started else {
            writer.cancelWriting()
            try? FileManager.default.removeItem(at: writer.outputURL)
            throw RecorderError.noFramesCaptured
        }

        videoInput?.markAsFinished()
        audioInput?.markAsFinished()
        microphoneInput?.markAsFinished()
        await writer.finishWriting()
        if writer.status == .failed {
            throw writer.error ?? RecorderError.noFramesCaptured
        }
    }

    // MARK: - SCStreamOutput

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard sampleBuffer.isValid, pausedAt == nil, let writer, writer.status == .writing else { return }

        switch type {
        case .screen:
            guard Self.isCompleteFrame(sampleBuffer), let buffer = retimed(sampleBuffer) else { return }
            if !sessionStarted {
                writer.startSession(atSourceTime: buffer.presentationTimeStamp)
                sessionStarted = true
            }
            if let videoInput, videoInput.isReadyForMoreMediaData {
                videoInput.append(buffer)
            }
        case .audio:
            guard sessionStarted, let audioInput, audioInput.isReadyForMoreMediaData,
                  let buffer = retimed(sampleBuffer) else { return }
            audioInput.append(buffer)
        default:
            // `.microphone` only exists from macOS 15, so it can't be its own case.
            if #available(macOS 15.0, *), type == .microphone {
                guard sessionStarted, let microphoneInput, microphoneInput.isReadyForMoreMediaData,
                      let buffer = retimed(sampleBuffer) else { return }
                microphoneInput.append(buffer)
            }
            break
        }
    }

    // MARK: - SCStreamDelegate

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        onStreamError?(error)
    }

    // MARK: - Helpers

    /// ScreenCaptureKit also delivers idle/blank status frames that carry no image.
    private static func isCompleteFrame(_ sampleBuffer: CMSampleBuffer) -> Bool {
        guard let attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false)
                as? [[SCStreamFrameInfo: Any]],
              let rawStatus = attachments.first?[.status] as? Int,
              let status = SCFrameStatus(rawValue: rawStatus) else { return false }
        return status == .complete
    }

    /// Shifts a sample back by the total time spent paused so the output has no gaps.
    private func retimed(_ sampleBuffer: CMSampleBuffer) -> CMSampleBuffer? {
        guard pauseOffset != .zero else { return sampleBuffer }

        var count: CMItemCount = 0
        CMSampleBufferGetSampleTimingInfoArray(sampleBuffer, entryCount: 0, arrayToFill: nil, entriesNeededOut: &count)
        var timing = [CMSampleTimingInfo](repeating: CMSampleTimingInfo(), count: count)
        CMSampleBufferGetSampleTimingInfoArray(sampleBuffer, entryCount: count, arrayToFill: &timing, entriesNeededOut: &count)

        for i in timing.indices {
            timing[i].presentationTimeStamp = CMTimeSubtract(timing[i].presentationTimeStamp, pauseOffset)
            if timing[i].decodeTimeStamp.isValid {
                timing[i].decodeTimeStamp = CMTimeSubtract(timing[i].decodeTimeStamp, pauseOffset)
            }
        }

        var output: CMSampleBuffer?
        CMSampleBufferCreateCopyWithNewTiming(
            allocator: kCFAllocatorDefault,
            sampleBuffer: sampleBuffer,
            sampleTimingEntryCount: count,
            sampleTimingArray: &timing,
            sampleBufferOut: &output
        )
        return output
    }
}
