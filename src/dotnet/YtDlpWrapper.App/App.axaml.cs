using Avalonia;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Markup.Xaml;
using Avalonia.Threading;
using YtDlpWrapper.Services;
using YtDlpWrapper.ViewModels;
using YtDlpWrapper.Views;

namespace YtDlpWrapper;

public partial class App : Application
{
    private BackendClient? _backend;
    private bool _shutdownStarted;

    public override void Initialize() => AvaloniaXamlLoader.Load(this);

    public override void OnFrameworkInitializationCompleted()
    {
        if (ApplicationLifetime is IClassicDesktopStyleApplicationLifetime desktop)
        {
            MainWindow? window = null;
            var platform = PlatformServices.Create(() => window);
            _backend = new BackendClient(platform);
            var viewModel = new MainWindowViewModel(_backend, platform);
            window = new MainWindow { DataContext = viewModel };
            desktop.MainWindow = window;
            window.Closing += (_, eventArgs) =>
            {
                if (_shutdownStarted)
                {
                    return;
                }

                _shutdownStarted = true;
                eventArgs.Cancel = true;
                window.Hide();
                _ = FinishShutdownAsync(desktop);
            };
        }

        base.OnFrameworkInitializationCompleted();
    }

    private async Task FinishShutdownAsync(IClassicDesktopStyleApplicationLifetime desktop)
    {
        try
        {
            if (_backend is not null)
            {
                await Task.Run(async () => await _backend.DisposeAsync());
            }
        }
        catch (Exception)
        {
            // The frontend is already hidden and the backend owns no persistent
            // user data that requires shutdown to complete successfully.
        }
        finally
        {
            await Dispatcher.UIThread.InvokeAsync(() => desktop.Shutdown());
        }
    }
}
