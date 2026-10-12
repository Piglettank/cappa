using System;
using System.Linq;
using System.Threading.Tasks;

namespace Cappa.Editor;

internal static partial class Exporter
{
    public static Task SaveVideoAsync(EditorModel model, string outputPath, IProgress<double> progress) =>
        SaveVideoAsync(model.SourcePath, model.Segments,
            model.AudioTracks.Select(t => new ExportAudio(t.StreamIndex, t.EffectiveVolume)).ToList(), outputPath, progress);

    public static Task SaveGifAsync(EditorModel model, string outputPath, IProgress<double> progress) =>
        SaveGifAsync(model.SourcePath, model.Segments, outputPath, progress);
}
