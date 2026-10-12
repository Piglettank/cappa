import AVFoundation
import ImageIO
import UniformTypeIdentifiers

enum ExportError: LocalizedError {
    case noVideoTrack
    case cannotCreateSession
    case cannotCreateGIF
    case failed(Error?)

    var errorDescription: String? {
        switch self {
        case .noVideoTrack: return "The recording has no video track."
        case .cannotCreateSession: return "The video could not be prepared for export."
        case .cannotCreateGIF: return "The GIF file could not be created."
        case .failed(let error): return error?.localizedDescription ?? "The export failed."
        }
    }
}

/// Joins the kept segments of a source video back to back and writes them out as a new file.
enum VideoExporter {
    static let gifFramesPerSecond = 20 // a 5 cs frame delay, which GIF can represent exactly
    static let gifMaximumSize = CGSize(width: 800, height: 800)

    static func exportVideo(
        asset: AVURLAsset,
        segments: [Segment],
        audioVolumes: [CMPersistentTrackID: Float],
        to outputURL: URL,
        progress: @escaping @MainActor (Double) -> Void
    ) async throws {
        let (composition, isHEVC, audioTrackIDs) = try await makeComposition(asset: asset, segments: segments)
        let preset = isHEVC ? AVAssetExportPresetHEVCHighestQuality : AVAssetExportPresetHighestQuality

        // Volumes are keyed by source track; translate them to the composition's tracks.
        var volumes: [CMPersistentTrackID: Float] = [:]
        for (source, target) in audioTrackIDs {
            volumes[target] = audioVolumes[source] ?? 1
        }
        for track in composition.tracks(withMediaType: .audio) where volumes[track.trackID] == 0 {
            composition.removeTrack(track)
        }

        // An export session keeps every audio track separate, and most players only play the first.
        // So computer audio and microphone are mixed into one track first, at their chosen levels.
        let mixedAudioURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("cappa-mix-\(UUID().uuidString).m4a")
        defer { try? FileManager.default.removeItem(at: mixedAudioURL) }
        let audioTracks = composition.tracks(withMediaType: .audio)
        if audioTracks.count > 1 || audioTracks.contains(where: { (volumes[$0.trackID] ?? 1) != 1 }) {
            try await mixAudio(of: composition, tracks: audioTracks, volumes: volumes, to: mixedAudioURL)
            audioTracks.forEach(composition.removeTrack)
            let mixedAsset = AVURLAsset(url: mixedAudioURL)
            if let mixed = try await mixedAsset.loadTracks(withMediaType: .audio).first,
               let target = composition.addMutableTrack(withMediaType: .audio, preferredTrackID: kCMPersistentTrackID_Invalid) {
                let range = try await mixed.load(.timeRange)
                try target.insertTimeRange(range, of: mixed, at: range.start)
            }
        }

        guard let session = AVAssetExportSession(asset: composition, presetName: preset) else {
            throw ExportError.cannotCreateSession
        }
        session.outputURL = outputURL
        session.outputFileType = .mp4
        session.shouldOptimizeForNetworkUse = true
        try? FileManager.default.removeItem(at: outputURL)

        let progressTask = Task { @MainActor in
            while !Task.isCancelled {
                progress(Double(session.progress))
                try? await Task.sleep(nanoseconds: 100_000_000)
            }
        }
        defer { progressTask.cancel() }

        await session.export()
        guard session.status == .completed else {
            throw ExportError.failed(session.error)
        }
    }

    /// A looping GIF at `gifFramesPerSecond`, scaled down to fit `gifMaximumSize`. Audio is dropped.
    static func exportGIF(
        asset: AVURLAsset,
        segments: [Segment],
        to outputURL: URL,
        progress: @escaping @MainActor (Double) -> Void
    ) async throws {
        let (composition, _, _) = try await makeComposition(asset: asset, segments: segments)
        let duration = composition.duration.seconds
        let frameDuration = 1 / Double(gifFramesPerSecond)
        let frameCount = max(Int((duration / frameDuration).rounded(.down)), 1)

        let generator = AVAssetImageGenerator(asset: composition)
        generator.appliesPreferredTrackTransform = true
        generator.maximumSize = gifMaximumSize
        let tolerance = CMTime(seconds: frameDuration / 2, preferredTimescale: 600)
        generator.requestedTimeToleranceBefore = tolerance
        generator.requestedTimeToleranceAfter = tolerance

        try? FileManager.default.removeItem(at: outputURL)
        guard let destination = CGImageDestinationCreateWithURL(
            outputURL as CFURL, UTType.gif.identifier as CFString, frameCount, nil
        ) else {
            throw ExportError.cannotCreateGIF
        }
        CGImageDestinationSetProperties(destination, [
            kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFLoopCount: 0],
        ] as CFDictionary)
        let frameProperties = [
            kCGImagePropertyGIFDictionary: [
                kCGImagePropertyGIFDelayTime: frameDuration,
                kCGImagePropertyGIFUnclampedDelayTime: frameDuration,
            ],
        ] as CFDictionary

        let times = (0..<frameCount).map { CMTime(seconds: Double($0) * frameDuration, preferredTimescale: 600) }
        var written = 0
        var previous: CGImage?
        for await result in generator.images(for: times) {
            // Frames arrive in request order; repeat the previous one if a frame can't be decoded,
            // so the frame count (and timing) matches what the destination expects.
            guard let image = (try? result.image) ?? previous else { continue }
            previous = image
            CGImageDestinationAddImage(destination, image, frameProperties)
            written += 1
            if written % 5 == 0 {
                await progress(Double(written) / Double(frameCount))
            }
        }

        guard written > 0, CGImageDestinationFinalize(destination) else {
            try? FileManager.default.removeItem(at: outputURL)
            throw ExportError.cannotCreateGIF
        }
    }

    // MARK: - Private

    /// Mixes audio tracks down to one stereo AAC file, each at its own volume.
    private static func mixAudio(
        of asset: AVAsset,
        tracks: [AVAssetTrack],
        volumes: [CMPersistentTrackID: Float],
        to url: URL
    ) async throws {
        let reader = try AVAssetReader(asset: asset)
        // Only touched on the writer's media queue once reading starts.
        nonisolated(unsafe) let output = AVAssetReaderAudioMixOutput(audioTracks: tracks, audioSettings: [
            AVFormatIDKey: kAudioFormatLinearPCM,
            AVSampleRateKey: 48_000,
            AVNumberOfChannelsKey: 2,
            AVLinearPCMBitDepthKey: 32,
            AVLinearPCMIsFloatKey: true,
            AVLinearPCMIsBigEndianKey: false,
            AVLinearPCMIsNonInterleaved: false,
        ])
        guard reader.canAdd(output) else { throw ExportError.cannotCreateSession }
        let mix = AVMutableAudioMix()
        mix.inputParameters = tracks.map { track in
            let parameters = AVMutableAudioMixInputParameters(track: track)
            parameters.setVolume(volumes[track.trackID] ?? 1, at: .zero)
            return parameters
        }
        output.audioMix = mix
        reader.add(output)

        let writer = try AVAssetWriter(outputURL: url, fileType: .m4a)
        nonisolated(unsafe) let input = AVAssetWriterInput(mediaType: .audio, outputSettings: [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 48_000,
            AVNumberOfChannelsKey: 2,
            AVEncoderBitRateKey: 192_000,
        ])
        guard writer.canAdd(input) else { throw ExportError.cannotCreateSession }
        writer.add(input)

        guard reader.startReading(), writer.startWriting() else {
            throw ExportError.failed(reader.error ?? writer.error)
        }
        writer.startSession(atSourceTime: .zero)

        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            input.requestMediaDataWhenReady(on: DispatchQueue(label: "dev.fallberg.cappa.audio-mix")) {
                while input.isReadyForMoreMediaData {
                    guard let buffer = output.copyNextSampleBuffer() else {
                        input.markAsFinished()
                        continuation.resume()
                        return
                    }
                    input.append(buffer)
                }
            }
        }
        await writer.finishWriting()

        guard reader.status == .completed, writer.status == .completed else {
            throw ExportError.failed(reader.error ?? writer.error)
        }
    }

    private static func makeComposition(
        asset: AVURLAsset,
        segments: [Segment]
    ) async throws -> (AVMutableComposition, isHEVC: Bool, audioTrackIDs: [CMPersistentTrackID: CMPersistentTrackID]) {
        guard let sourceVideo = try await asset.loadTracks(withMediaType: .video).first else {
            throw ExportError.noVideoTrack
        }
        let composition = AVMutableComposition()
        guard let video = composition.addMutableTrack(withMediaType: .video, preferredTrackID: kCMPersistentTrackID_Invalid) else {
            throw ExportError.cannotCreateSession
        }
        video.preferredTransform = try await sourceVideo.load(.preferredTransform)

        // Recordings can have two audio tracks (system audio and microphone); keep them all.
        var audioTracks: [(source: AVAssetTrack, target: AVMutableCompositionTrack, range: CMTimeRange)] = []
        var audioTrackIDs: [CMPersistentTrackID: CMPersistentTrackID] = [:] // source → composition
        for source in try await asset.loadTracks(withMediaType: .audio) {
            guard let target = composition.addMutableTrack(withMediaType: .audio, preferredTrackID: kCMPersistentTrackID_Invalid) else {
                continue
            }
            audioTracks.append((source, target, try await source.load(.timeRange)))
            audioTrackIDs[source.trackID] = target.trackID
        }

        var cursor = CMTime.zero
        for segment in segments {
            let range = CMTimeRange(
                start: CMTime(seconds: segment.start, preferredTimescale: 600),
                end: CMTime(seconds: segment.end, preferredTimescale: 600)
            )
            try video.insertTimeRange(range, of: sourceVideo, at: cursor)

            // Audio can start slightly later or end earlier than video, so only insert the overlap.
            for audio in audioTracks {
                let overlap = range.intersection(audio.range)
                if !overlap.isEmpty {
                    try audio.target.insertTimeRange(overlap, of: audio.source, at: CMTimeAdd(cursor, CMTimeSubtract(overlap.start, range.start)))
                }
            }
            cursor = CMTimeAdd(cursor, range.duration)
        }

        let formats = try await sourceVideo.load(.formatDescriptions)
        let isHEVC = formats.contains { CMFormatDescriptionGetMediaSubType($0) == kCMVideoCodecType_HEVC }
        return (composition, isHEVC, audioTrackIDs)
    }
}
