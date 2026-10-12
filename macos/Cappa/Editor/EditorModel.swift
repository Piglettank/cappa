import AVFoundation
import AppKit

/// A kept part of the source video, in source-time seconds.
struct Segment: Identifiable, Equatable {
    let id: UUID
    var start: Double
    var end: Double

    init(id: UUID = UUID(), start: Double, end: Double) {
        self.id = id
        self.start = start
        self.end = end
    }

    var duration: Double { end - start }

    func contains(_ time: Double) -> Bool { time >= start && time < end }
}

enum TrimEdge { case start, end }

/// One audio track of the source video. Audio tracks always follow the video's clips;
/// only their level is adjusted independently.
struct AudioTrack: Identifiable, Equatable {
    enum Kind { case computer, microphone }

    let id: CMPersistentTrackID
    let kind: Kind
    let name: String
    var volume: Float = 1
    var isMuted = false

    var effectiveVolume: Float { isMuted ? 0 : volume }
}

/// What an export captures, used to tell whether there are unexported changes.
private struct EditState: Equatable {
    let segments: [Segment]
    let volumes: [CMPersistentTrackID: Float]
}

/// Editing state for one recording. The edit is a list of kept segments in source order;
/// everything between them is cut. The source file is never modified, and export writes a new file.
@MainActor
final class EditorModel: ObservableObject {
    static let minimumSegmentDuration = 0.1
    /// The player's time grid (600 = common multiple of 24/25/30/60 fps). Clip edges are kept on it,
    /// so seeking to a clip's start lands exactly there instead of a rounding error before it.
    static let timescale: CMTimeScale = 600
    /// How far before a clip's start a reported player time still counts as inside the clip.
    private static let boundaryTolerance = 0.002

    static func snapped(_ time: Double) -> Double {
        (time * Double(timescale)).rounded() / Double(timescale)
    }

    let url: URL
    let player: AVPlayer
    private let asset: AVURLAsset

    @Published private(set) var isLoaded = false
    @Published private(set) var duration: Double = 0
    @Published private(set) var segments: [Segment] = [] {
        didSet { rebuildBoundaryObserver() }
    }
    @Published var selectedSegmentID: Segment.ID?
    @Published private(set) var currentTime: Double = 0
    @Published private(set) var isPlaying = false
    @Published private(set) var thumbnails: [CGImage?] = []
    @Published private(set) var audioTracks: [AudioTrack] = [] {
        didSet { applyAudioMix() }
    }
    /// Peak levels (0...1) per audio track, evenly spaced over the whole source.
    @Published private(set) var waveforms: [CMPersistentTrackID: [Float]] = [:]
    /// Non-nil while an export is running.
    @Published private(set) var exportProgress: Double?
    @Published private(set) var exportTitle = ""
    /// A short confirmation shown over the video, e.g. "Copied to clipboard".
    @Published private(set) var notice: String?
    @Published private(set) var canUndo = false
    @Published private(set) var canRedo = false

    private var undoStack: [[Segment]] = []
    private var redoStack: [[Segment]] = []
    private var exportedState: EditState?
    private var isJumpingOverCut = false
    private var periodicObserver: Any?
    private var boundaryObserver: Any?
    private var endObserver: NSObjectProtocol?

    init(url: URL) {
        self.url = url
        asset = AVURLAsset(url: url)
        player = AVPlayer(playerItem: AVPlayerItem(asset: asset))
        player.actionAtItemEnd = .pause
    }

    var editedDuration: Double { segments.reduce(0) { $0 + $1.duration } }

    var hasEdits: Bool {
        let isTrimmed = !(segments.count == 1 && segments[0].start == 0 && segments[0].end == duration)
        return isTrimmed || audioTracks.contains { $0.effectiveVolume != 1 }
    }

    var hasUnexportedEdits: Bool { hasEdits && editState != exportedState }

    /// Effective playback/export volume per source audio track.
    var audioVolumes: [CMPersistentTrackID: Float] {
        Dictionary(uniqueKeysWithValues: audioTracks.map { ($0.id, $0.effectiveVolume) })
    }

    private var editState: EditState { EditState(segments: segments, volumes: audioVolumes) }

    var selectedSegment: Segment? { segments.first { $0.id == selectedSegmentID } }

    var canSplit: Bool {
        guard let segment = segments.first(where: { $0.contains(currentTime) }) else { return false }
        return currentTime - segment.start >= Self.minimumSegmentDuration
            && segment.end - currentTime >= Self.minimumSegmentDuration
    }

    var canDelete: Bool { segments.count > 1 && selectedSegment != nil }

    // MARK: - Lifecycle

    func load() async {
        let loaded = (try? await asset.load(.duration))?.seconds ?? 0
        duration = loaded.isFinite ? loaded : 0
        audioTracks = await Self.describeAudioTracks(of: asset)
        segments = [Segment(start: 0, end: duration)]
        isLoaded = true
        observePlayer()
        await generateThumbnails()
        await generateWaveforms()
    }

    func close() {
        player.pause()
        if let periodicObserver { player.removeTimeObserver(periodicObserver) }
        if let boundaryObserver { player.removeTimeObserver(boundaryObserver) }
        if let endObserver { NotificationCenter.default.removeObserver(endObserver) }
        periodicObserver = nil
        boundaryObserver = nil
        endObserver = nil
    }

    // MARK: - Playback

    func togglePlay() {
        isPlaying ? pause() : play()
    }

    func play() {
        guard let first = segments.first else { return }
        // Starting in a cut part (or at the very end) jumps to the next kept part, wrapping to the start.
        if segment(containing: currentTime) == nil {
            let next = segments.first { $0.start >= currentTime } ?? first
            seek(to: next.start)
        }
        player.play()
        isPlaying = true
    }

    func pause() {
        player.pause()
        isPlaying = false
    }

    func seek(to time: Double) {
        let clamped = min(max(time, 0), duration)
        currentTime = clamped
        player.seek(to: CMTime(seconds: clamped, preferredTimescale: Self.timescale), toleranceBefore: .zero, toleranceAfter: .zero)
    }

    // MARK: - Editing

    /// Splits the clip under the playhead into two clips.
    func split() {
        let time = Self.snapped(currentTime)
        guard canSplit, let index = segments.firstIndex(where: { $0.contains(time) }) else { return }
        recordUndo()
        let original = segments[index]
        segments[index].end = time
        segments.insert(Segment(start: time, end: original.end), at: index + 1)
        selectedSegmentID = segments[index + 1].id
    }

    func deleteSelected() {
        guard canDelete, let index = segments.firstIndex(where: { $0.id == selectedSegmentID }) else { return }
        recordUndo()
        segments.remove(at: index)
        selectedSegmentID = nil
    }

    func beginTrim(_ id: Segment.ID) {
        pause()
        selectedSegmentID = id
        recordUndo()
    }

    /// Moves one edge of a clip. Clips can't overlap their neighbours or shrink below the minimum length.
    func trim(_ id: Segment.ID, edge: TrimEdge, to time: Double) {
        let time = Self.snapped(time)
        guard let index = segments.firstIndex(where: { $0.id == id }) else { return }
        switch edge {
        case .start:
            let lower = index > 0 ? segments[index - 1].end : 0
            segments[index].start = min(max(time, lower), segments[index].end - Self.minimumSegmentDuration)
            seek(to: segments[index].start)
        case .end:
            let upper = index < segments.count - 1 ? segments[index + 1].start : duration
            segments[index].end = max(min(time, upper), segments[index].start + Self.minimumSegmentDuration)
            seek(to: segments[index].end)
        }
    }

    func endTrim() {
        // Drop the undo step if the drag didn't actually change anything.
        if undoStack.last == segments {
            undoStack.removeLast()
            updateUndoState()
        }
    }

    func undo() {
        guard let previous = undoStack.popLast() else { return }
        redoStack.append(segments)
        segments = previous
        clearStaleSelection()
        updateUndoState()
    }

    func redo() {
        guard let next = redoStack.popLast() else { return }
        undoStack.append(segments)
        segments = next
        clearStaleSelection()
        updateUndoState()
    }

    // MARK: - Audio

    func setVolume(_ volume: Float, for id: CMPersistentTrackID) {
        guard let index = audioTracks.firstIndex(where: { $0.id == id }) else { return }
        audioTracks[index].volume = min(max(volume, 0), 1)
    }

    func toggleMute(_ id: CMPersistentTrackID) {
        guard let index = audioTracks.firstIndex(where: { $0.id == id }) else { return }
        audioTracks[index].isMuted.toggle()
    }

    // MARK: - Export

    func saveVideo(to outputURL: URL) async throws {
        let volumes = audioVolumes
        try await runExport("Exporting video…") { snapshot, progress in
            try await VideoExporter.exportVideo(
                asset: self.asset, segments: snapshot, audioVolumes: volumes, to: outputURL, progress: progress
            )
        }
    }

    func saveGIF(to outputURL: URL) async throws {
        try await runExport("Creating GIF…") { snapshot, progress in
            try await VideoExporter.exportGIF(asset: self.asset, segments: snapshot, to: outputURL, progress: progress)
        }
    }

    /// Puts the edited video on the clipboard as a file, so it pastes into Finder, Slack, Messages, Mail, etc.
    func copyToClipboard() async throws {
        var fileURL = url
        // Recordings with computer audio and mic have two audio tracks, and many apps only play the first,
        // so those are exported (which mixes them) even without edits.
        if hasEdits || audioTracks.count > 1 {
            let folder = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
                .appendingPathComponent(Bundle.main.bundleIdentifier ?? "Cappa", isDirectory: true)
                .appendingPathComponent("Clipboard", isDirectory: true)
            // Only the most recent copy needs to stay around for pasting.
            try? FileManager.default.removeItem(at: folder)
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            fileURL = folder.appendingPathComponent(url.deletingPathExtension().lastPathComponent + " (edited).mp4")
            // A clipboard copy is temporary, so it doesn't count as exported for the close warning.
            let exported = exportedState
            try await saveVideo(to: fileURL)
            exportedState = exported
        }

        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.writeObjects([fileURL as NSURL])
        showNotice("Copied to clipboard")
    }

    // MARK: - Private

    private func runExport(
        _ title: String,
        _ work: @escaping ([Segment], @escaping @MainActor (Double) -> Void) async throws -> Void
    ) async throws {
        pause()
        let state = editState
        exportTitle = title
        exportProgress = 0
        defer { exportProgress = nil }

        try await work(state.segments) { [weak self] progress in
            self?.exportProgress = progress
        }
        exportedState = state
    }

    private func showNotice(_ text: String) {
        notice = text
        Task {
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            if notice == text { notice = nil }
        }
    }

    /// Playback applies each track's level live through the player item's audio mix.
    private func applyAudioMix() {
        let mix = AVMutableAudioMix()
        mix.inputParameters = audioTracks.map { track in
            let parameters = AVMutableAudioMixInputParameters()
            parameters.trackID = track.id
            parameters.setVolume(track.effectiveVolume, at: .zero)
            return parameters
        }
        player.currentItem?.audioMix = mix
    }

    /// Cappa records computer audio in stereo and the microphone in mono, which is how the tracks are told apart.
    private static func describeAudioTracks(of asset: AVAsset) async -> [AudioTrack] {
        guard let tracks = try? await asset.loadTracks(withMediaType: .audio) else { return [] }
        var result: [AudioTrack] = []
        for track in tracks {
            let formats = (try? await track.load(.formatDescriptions)) ?? []
            let channels = formats.first
                .flatMap { CMAudioFormatDescriptionGetStreamBasicDescription($0)?.pointee.mChannelsPerFrame } ?? 2
            if channels == 1 {
                result.append(AudioTrack(id: track.trackID, kind: .microphone, name: "Microphone"))
            } else {
                result.append(AudioTrack(id: track.trackID, kind: .computer, name: tracks.count > 1 ? "Computer Audio" : "Audio"))
            }
        }
        return result
    }

    private func generateWaveforms() async {
        let url = url
        let duration = duration
        for track in audioTracks {
            let id = track.id
            let levels = await Task.detached(priority: .utility) {
                await Self.peakLevels(url: url, trackID: id, duration: duration)
            }.value
            waveforms[id] = levels
        }
    }

    /// Reads a track as 8 kHz mono PCM and keeps the peak of each bucket, scaled to a 50 dB range.
    private nonisolated static func peakLevels(url: URL, trackID: CMPersistentTrackID, duration: Double) async -> [Float] {
        let bucketCount = 800
        let sampleRate = 8_000.0
        guard duration > 0 else { return [] }

        let asset = AVURLAsset(url: url)
        guard let track = try? await asset.loadTrack(withTrackID: trackID), let reader = try? AVAssetReader(asset: asset) else { return [] }

        let output = AVAssetReaderTrackOutput(track: track, outputSettings: [
            AVFormatIDKey: kAudioFormatLinearPCM,
            AVSampleRateKey: sampleRate,
            AVNumberOfChannelsKey: 1,
            AVLinearPCMBitDepthKey: 32,
            AVLinearPCMIsFloatKey: true,
            AVLinearPCMIsBigEndianKey: false,
            AVLinearPCMIsNonInterleaved: false,
        ])
        guard reader.canAdd(output) else { return [] }
        reader.add(output)
        guard reader.startReading() else { return [] }

        var peaks = [Float](repeating: 0, count: bucketCount)
        while let buffer = output.copyNextSampleBuffer() {
            guard let block = CMSampleBufferGetDataBuffer(buffer) else { continue }
            let start = CMSampleBufferGetPresentationTimeStamp(buffer).seconds
            var length = 0
            var pointer: UnsafeMutablePointer<CChar>?
            guard CMBlockBufferGetDataPointer(block, atOffset: 0, lengthAtOffsetOut: &length, totalLengthOut: nil, dataPointerOut: &pointer) == noErr,
                  let pointer else { continue }
            let count = length / MemoryLayout<Float>.size
            pointer.withMemoryRebound(to: Float.self, capacity: count) { samples in
                for i in 0..<count {
                    let time = start + Double(i) / sampleRate
                    let bucket = min(max(Int(time / duration * Double(bucketCount)), 0), bucketCount - 1)
                    peaks[bucket] = max(peaks[bucket], abs(samples[i]))
                }
            }
        }

        return peaks.map { peak in
            guard peak > 0 else { return 0 }
            return max(0, 1 + 20 * log10(peak) / 50)
        }
    }

    /// The clip at `time`, allowing for the player reporting a time a hair before a clip's start.
    private func segment(containing time: Double) -> Segment? {
        segments.first { time >= $0.start - Self.boundaryTolerance && time < $0.end }
    }

    private func recordUndo() {
        undoStack.append(segments)
        redoStack.removeAll()
        updateUndoState()
    }

    private func updateUndoState() {
        canUndo = !undoStack.isEmpty
        canRedo = !redoStack.isEmpty
    }

    private func clearStaleSelection() {
        if selectedSegment == nil { selectedSegmentID = nil }
    }

    private func observePlayer() {
        let interval = CMTime(value: 1, timescale: 30)
        periodicObserver = player.addPeriodicTimeObserver(forInterval: interval, queue: .main) { [weak self] time in
            Task { @MainActor in self?.playerDidReach(time.seconds) }
        }
        endObserver = NotificationCenter.default.addObserver(
            forName: .AVPlayerItemDidPlayToEndTime, object: player.currentItem, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in self?.isPlaying = false }
        }
        rebuildBoundaryObserver()
    }

    /// Fires exactly at each clip's end so playback skips cut parts without showing them.
    private func rebuildBoundaryObserver() {
        if let boundaryObserver { player.removeTimeObserver(boundaryObserver) }
        boundaryObserver = nil
        guard periodicObserver != nil, !segments.isEmpty else { return }

        let times = segments.map { NSValue(time: CMTime(seconds: $0.end, preferredTimescale: Self.timescale)) }
        boundaryObserver = player.addBoundaryTimeObserver(forTimes: times, queue: .main) { [weak self] in
            Task { @MainActor in
                guard let self else { return }
                self.playerDidReach(self.player.currentTime().seconds)
            }
        }
    }

    private func playerDidReach(_ time: Double) {
        // While a jump is in flight the player still reports times inside the cut part;
        // reacting to those would re-issue the seek, cancel the one in flight and stall playback.
        guard isPlaying, !isJumpingOverCut, time.isFinite else { return }
        currentTime = time
        guard segment(containing: time) == nil else { return }

        if let next = segments.first(where: { $0.start >= time - 0.001 }) {
            jumpOverCut(to: next.start)
        } else {
            pause()
            seek(to: segments.last?.end ?? duration)
        }
    }

    /// Seeks past a cut part and keeps playing once the seek lands.
    private func jumpOverCut(to time: Double) {
        isJumpingOverCut = true
        currentTime = time
        player.seek(
            to: CMTime(seconds: time, preferredTimescale: Self.timescale), toleranceBefore: .zero, toleranceAfter: .zero
        ) { [weak self] _ in
            Task { @MainActor in
                guard let self else { return }
                self.isJumpingOverCut = false
                if self.isPlaying && self.player.rate == 0 {
                    self.player.play()
                }
            }
        }
    }

    private func generateThumbnails() async {
        guard duration > 0 else { return }
        let count = 24
        let generator = AVAssetImageGenerator(asset: asset)
        generator.appliesPreferredTrackTransform = true
        generator.maximumSize = CGSize(width: 320, height: 200)
        let tolerance = CMTime(seconds: duration / Double(count) / 2, preferredTimescale: Self.timescale)
        generator.requestedTimeToleranceBefore = tolerance
        generator.requestedTimeToleranceAfter = tolerance

        let times = (0..<count).map {
            CMTime(seconds: duration * (Double($0) + 0.5) / Double(count), preferredTimescale: Self.timescale)
        }
        thumbnails = Array(repeating: nil, count: count)
        for await result in generator.images(for: times) {
            guard let image = try? result.image, let index = times.firstIndex(of: result.requestedTime) else { continue }
            thumbnails[index] = image
        }
    }
}
