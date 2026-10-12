using System;
using System.Threading;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Threading;
using Cappa.Recording;
using Cappa.Services;
using Cappa.UI;

namespace Cappa;

/// <summary>
/// Cappa runs in the notification area with no main window. Recording starts from the tray menu or
/// Ctrl+Alt+Shift+R; finished recordings open in the editor.
/// </summary>
public partial class App : Application
{
    private Mutex? _singleInstance;
    private RecordingController? _controller;
    private TrayIcon? _trayIcon;
    private GlobalHotKey? _hotKey;

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);

        _singleInstance = new Mutex(initiallyOwned: true, "Cappa.SingleInstance", out var isFirstInstance);
        if (!isFirstInstance)
        {
            MessageBox.Show("Cappa is already running. Look for its icon in the notification area.", "Cappa");
            Shutdown();
            return;
        }

        DispatcherUnhandledException += OnUnhandledException;
        TaskScheduler.UnobservedTaskException += (_, args) =>
        {
            Log.Error("Unobserved task exception", args.Exception);
            args.SetObserved();
        };
        Log.Write($"Cappa {typeof(App).Assembly.GetName().Version} starting on {Environment.OSVersion}");

        var settings = AppSettings.Load();
        _controller = new RecordingController(settings);
        _trayIcon = new TrayIcon(_controller);
        _hotKey = GlobalHotKey.RegisterRecordShortcut(() => _ = _controller.ToggleRecordingAsync());

        _trayIcon.ShowBalloon("Cappa is running",
            _hotKey is null
                ? "Click the Cappa icon in the notification area to record."
                : "Press Ctrl+Alt+Shift+R or click the Cappa icon to record.");

        // "Cappa.exe path\to\video.mp4" opens the video straight in the editor.
        if (e.Args.Length > 0 && System.IO.File.Exists(e.Args[0]))
        {
            Editor.EditorWindow.Open(e.Args[0]);
        }
    }

    protected override void OnExit(ExitEventArgs e)
    {
        _hotKey?.Dispose();
        _trayIcon?.Dispose();
        _singleInstance?.Dispose();
        Log.Write("Cappa exiting");
        base.OnExit(e);
    }

    private void OnUnhandledException(object sender, DispatcherUnhandledExceptionEventArgs e)
    {
        Log.Error("Unhandled exception", e.Exception);
        MessageBox.Show($"Something went wrong:\n\n{e.Exception.Message}\n\nDetails were written to {Log.FilePath}.",
            "Cappa", MessageBoxButton.OK, MessageBoxImage.Error);
        e.Handled = true;
    }
}
