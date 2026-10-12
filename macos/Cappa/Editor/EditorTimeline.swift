import SwiftUI

/// The timeline maps the whole source recording at a fixed scale. Kept clips are drawn bright,
/// and the parts that are cut away stay visible but dimmed, so you can drag an edge back to restore them.
/// Audio tracks get their own lanes below the video; they follow the video's clips and only their level is separate.
struct EditorTimeline: View {
    @ObservedObject var model: EditorModel

    private static let rulerHeight: CGFloat = 22
    private static let videoHeight: CGFloat = 72
    private static let audioHeight: CGFloat = 38
    private static let laneSpacing: CGFloat = 6
    private static let headerWidth: CGFloat = 170
    private static let handleWidth: CGFloat = 12
    private static let space = "timeline"

    /// The clip edge being dragged, and how far from the exact edge the drag began.
    @State private var activeTrim: (id: Segment.ID, grabOffset: CGFloat)?

    private var totalHeight: CGFloat {
        Self.rulerHeight + Self.laneSpacing + Self.videoHeight
            + CGFloat(model.audioTracks.count) * (Self.laneSpacing + Self.audioHeight)
    }

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            headers
                .frame(width: Self.headerWidth)

            GeometryReader { geometry in
                let width = geometry.size.width

                ZStack(alignment: .topLeading) {
                    VStack(alignment: .leading, spacing: Self.laneSpacing) {
                        ruler(width: width)
                            .frame(width: width, height: Self.rulerHeight)
                            .contentShape(Rectangle())
                            .gesture(scrubGesture(width: width))

                        videoLane(width: width)
                            .frame(width: width, height: Self.videoHeight, alignment: .topLeading)

                        ForEach(model.audioTracks) { track in
                            audioLane(track, width: width)
                                .frame(width: width, height: Self.audioHeight)
                        }
                    }

                    playhead(width: width, height: totalHeight)
                }
                .coordinateSpace(name: Self.space)
            }
        }
        .frame(height: totalHeight)
    }

    // MARK: - Lane headers

    private var headers: some View {
        VStack(alignment: .leading, spacing: Self.laneSpacing) {
            Color.clear.frame(height: Self.rulerHeight)

            Label("Video", systemImage: "film")
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: Self.videoHeight, alignment: .leading)

            ForEach(model.audioTracks) { track in
                audioHeader(track)
                    .frame(height: Self.audioHeight)
            }
        }
    }

    private func audioHeader(_ track: AudioTrack) -> some View {
        HStack(spacing: 6) {
            Button {
                model.toggleMute(track.id)
            } label: {
                Image(systemName: track.isMuted
                      ? "speaker.slash.fill"
                      : track.kind == .microphone ? "mic.fill" : "speaker.wave.2.fill")
                    .foregroundStyle(track.isMuted ? Color.secondary : Self.color(for: track))
                    .frame(width: 20, height: 20)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(track.isMuted ? "Unmute \(track.name)" : "Mute \(track.name)")

            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 4) {
                    Text(track.name)
                        .font(.caption)
                        .lineLimit(1)
                    Spacer(minLength: 2)
                    Text(track.isMuted ? "Muted" : "\(Int((track.volume * 100).rounded()))%")
                        .font(.caption2.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
                Slider(
                    value: Binding(
                        get: { Double(track.volume) },
                        set: { model.setVolume(Float($0), for: track.id) }
                    ),
                    in: 0...1
                )
                .controlSize(.mini)
                .opacity(track.isMuted ? 0.4 : 1)
            }
        }
    }

    // MARK: - Lanes

    private func videoLane(width: CGFloat) -> some View {
        ZStack(alignment: .topLeading) {
            Filmstrip(images: model.thumbnails)
                .frame(width: width, height: Self.videoHeight)
                .opacity(0.25)
                .clipShape(RoundedRectangle(cornerRadius: 6))
                .gesture(scrubGesture(width: width))

            ForEach(model.segments) { segment in
                clip(segment, width: width, trackHeight: Self.videoHeight)
            }
        }
    }

    /// The track's waveform, bright inside kept clips and dimmed in cut-away parts.
    /// Bar heights follow the track's volume, so turning it down is visible.
    private func audioLane(_ track: AudioTrack, width: CGFloat) -> some View {
        let levels = model.waveforms[track.id] ?? []
        let color = track.isMuted ? Color.gray : Self.color(for: track)
        let gain = CGFloat(track.isMuted ? 1 : track.volume)
        let segments = model.segments
        let selectedID = model.selectedSegmentID
        let duration = model.duration

        return Canvas { context, size in
            context.fill(Path(roundedRect: CGRect(origin: .zero, size: size), cornerRadius: 5),
                         with: .color(.primary.opacity(0.05)))
            guard duration > 0 else { return }

            for segment in segments {
                let x0 = CGFloat(segment.start / duration) * size.width
                let x1 = CGFloat(segment.end / duration) * size.width
                let rect = CGRect(x: x0, y: 0, width: max(x1 - x0, 2), height: size.height)
                context.fill(Path(roundedRect: rect, cornerRadius: 5), with: .color(color.opacity(0.18)))
                let isSelected = segment.id == selectedID
                context.stroke(Path(roundedRect: rect.insetBy(dx: 0.75, dy: 0.75), cornerRadius: 5),
                               with: .color(isSelected ? .yellow : color.opacity(0.6)),
                               lineWidth: isSelected ? 2 : 1)
            }

            guard !levels.isEmpty else { return }
            let mid = size.height / 2
            var x: CGFloat = 1
            while x < size.width - 1 {
                let fraction = Double(x / size.width)
                let level = CGFloat(levels[min(Int(fraction * Double(levels.count)), levels.count - 1)])
                let height = max(level * gain * (size.height - 8), 1)
                let isKept = segments.contains { $0.contains(fraction * duration) }
                context.fill(Path(CGRect(x: x, y: mid - height / 2, width: 1.5, height: height)),
                             with: .color(isKept ? color : color.opacity(0.25)))
                x += 2.5
            }
        }
        .contentShape(Rectangle())
        .gesture(
            // Like the video lane: clicking selects the clip under the pointer (or nothing) and scrubs.
            DragGesture(minimumDistance: 0, coordinateSpace: .named(Self.space))
                .onChanged { value in
                    let time = time(at: value.location.x, width: width)
                    model.pause()
                    model.selectedSegmentID = model.segments.first { $0.contains(time) }?.id
                    model.seek(to: time)
                }
        )
    }

    private static func color(for track: AudioTrack) -> Color {
        track.kind == .microphone ? .orange : .teal
    }

    // MARK: - Clips

    private func clip(_ segment: Segment, width: CGFloat, trackHeight: CGFloat) -> some View {
        let x = position(of: segment.start, width: width)
        let clipWidth = max(position(of: segment.end, width: width) - x, 2)
        let isSelected = segment.id == model.selectedSegmentID

        return ZStack {
            // The full filmstrip, shifted so this clip shows the frames it actually contains.
            Filmstrip(images: model.thumbnails)
                .frame(width: width, height: trackHeight)
                .offset(x: -x)
                .frame(width: clipWidth, height: trackHeight, alignment: .leading)
                .clipped()

            RoundedRectangle(cornerRadius: 6)
                .strokeBorder(isSelected ? Color.yellow : Color.white.opacity(0.7), lineWidth: isSelected ? 3 : 1.5)

            HStack(spacing: 0) {
                handle(segment, edge: .start, isSelected: isSelected, width: width)
                Spacer(minLength: 0)
                handle(segment, edge: .end, isSelected: isSelected, width: width)
            }
        }
        .frame(width: clipWidth, height: trackHeight)
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .contentShape(Rectangle())
        .offset(x: x)
        .gesture(
            // Clicking or dragging inside a clip selects it and scrubs.
            DragGesture(minimumDistance: 0, coordinateSpace: .named(Self.space))
                .onChanged { value in
                    model.pause()
                    model.selectedSegmentID = segment.id
                    model.seek(to: time(at: value.location.x, width: width))
                }
        )
    }

    private func handle(_ segment: Segment, edge: TrimEdge, isSelected: Bool, width: CGFloat) -> some View {
        ZStack {
            Rectangle()
                .fill(isSelected ? Color.yellow : Color.white.opacity(0.85))
            Image(systemName: edge == .start ? "chevron.compact.left" : "chevron.compact.right")
                .font(.system(size: 13, weight: .heavy))
                .foregroundColor(.black.opacity(0.65))
        }
        .frame(width: Self.handleWidth)
        .contentShape(Rectangle())
        .onHover { inside in
            if inside { NSCursor.resizeLeftRight.push() } else { NSCursor.pop() }
        }
        .gesture(
            DragGesture(minimumDistance: 0, coordinateSpace: .named(Self.space))
                .onChanged { value in
                    if activeTrim?.id != segment.id {
                        let edgeX = position(of: edge == .start ? segment.start : segment.end, width: width)
                        activeTrim = (segment.id, value.startLocation.x - edgeX)
                        model.beginTrim(segment.id)
                    }
                    let x = value.location.x - (activeTrim?.grabOffset ?? 0)
                    model.trim(segment.id, edge: edge, to: time(at: x, width: width))
                }
                .onEnded { _ in
                    activeTrim = nil
                    model.endTrim()
                }
        )
    }

    // MARK: - Ruler and playhead

    private func ruler(width: CGFloat) -> some View {
        let duration = model.duration
        return Canvas { context, size in
            guard duration > 0 else { return }
            let steps: [Double] = [0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1800]
            let step = steps.first { $0 / duration * Double(size.width) >= 64 } ?? 3600

            var t = 0.0
            while t <= duration {
                let x = CGFloat(t / duration) * size.width
                context.fill(Path(CGRect(x: x, y: size.height - 6, width: 1, height: 6)), with: .color(.secondary))
                context.draw(
                    Text(TimeFormat.short(t)).font(.system(size: 10).monospacedDigit()).foregroundColor(.secondary),
                    at: CGPoint(x: x + 3, y: size.height - 8),
                    anchor: .bottomLeading
                )
                t += step
            }
        }
    }

    private func playhead(width: CGFloat, height: CGFloat) -> some View {
        let x = position(of: model.currentTime, width: width)
        return VStack(spacing: 0) {
            Image(systemName: "arrowtriangle.down.fill")
                .font(.system(size: 10))
                .foregroundColor(.red)
            Rectangle()
                .fill(Color.red)
                .frame(width: 2)
        }
        .frame(width: 12, height: height)
        .offset(x: x - 6)
        .allowsHitTesting(false)
    }

    /// Scrubbing outside a clip (ruler or cut-away area) clears the selection.
    private func scrubGesture(width: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 0, coordinateSpace: .named(Self.space))
            .onChanged { value in
                model.pause()
                model.selectedSegmentID = nil
                model.seek(to: time(at: value.location.x, width: width))
            }
    }

    // MARK: - Geometry

    private func position(of time: Double, width: CGFloat) -> CGFloat {
        guard model.duration > 0 else { return 0 }
        return CGFloat(time / model.duration) * width
    }

    private func time(at x: CGFloat, width: CGFloat) -> Double {
        guard width > 0 else { return 0 }
        return Double(min(max(x, 0), width) / width) * model.duration
    }
}

private struct Filmstrip: View {
    let images: [CGImage?]

    var body: some View {
        GeometryReader { geometry in
            let tileWidth = images.isEmpty ? 0 : geometry.size.width / CGFloat(images.count)
            HStack(spacing: 0) {
                ForEach(images.indices, id: \.self) { index in
                    Group {
                        if let image = images[index] {
                            Image(decorative: image, scale: 1)
                                .resizable()
                                .scaledToFill()
                        } else {
                            Color.gray.opacity(0.3)
                        }
                    }
                    .frame(width: tileWidth, height: geometry.size.height)
                    .clipped()
                }
            }
        }
        .background(Color.black.opacity(0.5))
    }
}
