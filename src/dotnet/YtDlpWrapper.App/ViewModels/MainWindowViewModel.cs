using System.Text.Json;
using Avalonia.Threading;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using YtDlpWrapper.Services;

namespace YtDlpWrapper.ViewModels;

public partial class MainWindowViewModel : ObservableObject
{
    private readonly IBackendClient _backend;
    private readonly IPlatformServices _platform;
    private string? _activeOperationId;
    private bool _toolsReadyBeforeOperation;

    [ObservableProperty]
    private string _url = string.Empty;

    [ObservableProperty]
    private string _outputFolder = string.Empty;

    [ObservableProperty]
    private bool _audioOnly;

    [ObservableProperty]
    private double _videoQuality = 2;

    [ObservableProperty]
    private bool _busy = true;

    [ObservableProperty]
    private bool _cancellable;

    [ObservableProperty]
    private bool _toolsReady;

    [ObservableProperty]
    private bool _setupRequired;

    [ObservableProperty]
    private bool _updateAvailable;

    [ObservableProperty]
    private bool _canInstallTools;

    [ObservableProperty]
    private bool _completed;

    [ObservableProperty]
    private bool _failed;

    [ObservableProperty]
    private bool _showDetails;

    [ObservableProperty]
    private bool _showAbout;

    [ObservableProperty]
    private double _progress;

    [ObservableProperty]
    private string _statusText = "Starting the download engine…";

    [ObservableProperty]
    private string _updateSummary = string.Empty;

    [ObservableProperty]
    private string _detailsText = string.Empty;

    [ObservableProperty]
    private string? _completedPath;

    [ObservableProperty]
    private bool _engineUnavailable;

    public MainWindowViewModel(IBackendClient backend, IPlatformServices platform)
    {
        _backend = backend;
        _platform = platform;
        _backend.EventReceived += OnBackendEvent;
        _backend.BackendExited += OnBackendExited;
    }

    public bool CanEdit => ToolsReady && !Busy;
    public bool CanBrowse => !Busy;
    public bool CanChangeQuality => CanEdit && !AudioOnly;
    public bool CanDownload =>
        ToolsReady && !Busy && !UpdateAvailable &&
        !string.IsNullOrWhiteSpace(Url) && !string.IsNullOrWhiteSpace(OutputFolder);
    public bool ShowDownloadButton => !Busy;
    public bool ShowCancelButton => Busy && Cancellable;
    public bool ShowInstallPanel => SetupRequired;
    public bool ShowUpdatePanel => UpdateAvailable && !SetupRequired;
    public bool ShowDetailsButton => !string.IsNullOrWhiteSpace(DetailsText);
    public bool IsProgressIndeterminate => Busy && Progress <= 0;
    public bool HasCompletedFile => Completed && !string.IsNullOrWhiteSpace(CompletedPath);
    public string DetailsButtonText => ShowDetails ? "Hide details" : "Show details";
    public bool ShowRestartButton => EngineUnavailable && !Busy;

    public async Task InitializeAsync()
    {
        try
        {
            Busy = true;
            await _backend.StartAsync();
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(5));
            var initialized = await _backend.SendAsync("initialize", cancellationToken: timeout.Token);
            var backendVersion = initialized.GetProperty("backendVersion").GetString();
            var frontendVersion = typeof(MainWindowViewModel).Assembly.GetName().Version?.ToString(3);
            if (backendVersion != frontendVersion)
            {
                throw new InvalidDataException(
                    $"Frontend version {frontendVersion ?? "unknown"} cannot use backend version {backendVersion ?? "unknown"}.");
            }
            EngineUnavailable = false;
            OutputFolder = initialized.GetProperty("outputFolder").GetString() ?? string.Empty;
            await CheckToolsAsync();
        }
        catch (Exception error)
        {
            ApplyFailure("The download engine could not be started.", error);
            EngineUnavailable = true;
        }
    }

    [RelayCommand]
    private async Task RestartBackendAsync()
    {
        EngineUnavailable = false;
        Busy = true;
        StatusText = "Restarting the download engine…";
        try
        {
            await _backend.StopAsync();
        }
        catch (Exception)
        {
            // Initialization below reports a useful error if cleanup was incomplete.
        }
        await InitializeAsync();
    }

    [RelayCommand]
    private async Task BrowseOutputAsync()
    {
        try
        {
            var selected = await _platform.PickOutputFolderAsync(OutputFolder);
            if (selected is null)
            {
                return;
            }

            await _backend.SendAsync("setOutputFolder", new { path = selected });
            OutputFolder = selected;
        }
        catch (Exception error)
        {
            ApplyFailure("The output folder could not be saved.", error);
        }
    }

    [RelayCommand]
    private async Task CheckToolsAsync()
    {
        SetWorking("Checking required tools…", false);
        SetupRequired = false;
        UpdateAvailable = false;
        CanInstallTools = false;
        ToolsReady = false;
        try
        {
            var result = await _backend.SendAsync("checkTools");
            Busy = false;
            ToolsReady = result.GetProperty("toolsReady").GetBoolean();
            CanInstallTools = result.GetProperty("canInstallTools").GetBoolean();
            UpdateSummary = result.GetProperty("updateSummary").GetString() ?? string.Empty;
            StatusText = result.GetProperty("statusText").GetString() ?? string.Empty;
            var state = result.GetProperty("state").GetString();
            SetupRequired = state == "setupRequired";
            UpdateAvailable = state == "updateAvailable";
            Failed = false;
        }
        catch (Exception error)
        {
            ToolsReady = false;
            SetupRequired = true;
            CanInstallTools = false;
            ApplyFailure("Could not check required tools.", error);
            UpdateSummary = "An internet connection is required for the first setup.";
        }
        finally
        {
            RefreshComputedProperties();
        }
    }

    [RelayCommand]
    private async Task InstallToolsAsync()
    {
        _toolsReadyBeforeOperation = ToolsReady;
        SetWorking("Preparing tool installation…", true);
        ToolsReady = false;
        CanInstallTools = false;
        try
        {
            var result = await _backend.SendAsync("installTools");
            _activeOperationId = result.GetProperty("operationId").GetString();
        }
        catch (Exception error)
        {
            ToolsReady = _toolsReadyBeforeOperation;
            ApplyFailure("Tool installation failed.", error);
            SetupRequired = !ToolsReady;
            CanInstallTools = true;
        }
    }

    [RelayCommand]
    private void DeferUpdate()
    {
        UpdateAvailable = false;
        CanInstallTools = false;
        Busy = false;
        Cancellable = false;
        ToolsReady = true;
        StatusText = "Ready. The cached tools will be used.";
        RefreshComputedProperties();
    }

    [RelayCommand]
    private async Task StartDownloadAsync()
    {
        SetWorking("Preparing download…", true);
        Completed = false;
        CompletedPath = null;
        Progress = 0;
        try
        {
            var quality = (int)Math.Round(VideoQuality) switch
            {
                <= 0 => "p1080",
                1 => "p1440",
                _ => "best",
            };
            var result = await _backend.SendAsync("startDownload", new
            {
                url = Url,
                mode = AudioOnly ? "audioOnly" : "video",
                videoQuality = quality,
                outputFolder = OutputFolder,
            });
            _activeOperationId = result.GetProperty("operationId").GetString();
        }
        catch (Exception error)
        {
            ApplyFailure("The download failed.", error);
        }
    }

    [RelayCommand]
    private async Task CancelAsync()
    {
        if (_activeOperationId is null)
        {
            return;
        }

        Cancellable = false;
        StatusText = "Cancelling…";
        RefreshComputedProperties();
        try
        {
            await _backend.SendAsync("cancel", new { operationId = _activeOperationId });
        }
        catch (Exception error)
        {
            ApplyFailure("The operation could not be cancelled.", error);
        }
    }

    [RelayCommand]
    private void OpenFolder()
    {
        if (CompletedPath is not null)
        {
            _platform.RevealFile(CompletedPath);
        }
    }

    [RelayCommand]
    private void ToggleAbout() => ShowAbout = !ShowAbout;

    [RelayCommand]
    private void ToggleDetails() => ShowDetails = !ShowDetails;

    private void OnBackendEvent(BackendEvent message) =>
        Dispatcher.UIThread.Post(() => HandleBackendEvent(message));

    internal void HandleBackendEvent(BackendEvent message)
    {
        if (message.OperationId != _activeOperationId)
        {
            return;
        }

        var operationKind = message.Data.TryGetProperty("operationKind", out var kind)
            ? kind.GetString()
            : null;
        switch (message.Event)
        {
            case "operationProgress":
                StatusText = message.Data.GetProperty("message").GetString() ?? StatusText;
                if (message.Data.TryGetProperty("fraction", out var fraction) &&
                    fraction.ValueKind == JsonValueKind.Number)
                {
                    Progress = fraction.GetDouble() * 100;
                }
                break;
            case "operationCompleted" when operationKind == "toolInstall":
                FinishOperation();
                ToolsReady = true;
                _toolsReadyBeforeOperation = false;
                SetupRequired = false;
                UpdateAvailable = false;
                CanInstallTools = false;
                StatusText = "Tools installed. Ready to download.";
                break;
            case "operationCompleted" when operationKind == "download":
                FinishOperation();
                Completed = true;
                Progress = 100;
                CompletedPath = message.Data.GetProperty("path").GetString();
                StatusText = CompletedPath is null ? "Download complete." : $"Saved {CompletedPath}";
                break;
            case "operationCancelled":
                FinishOperation();
                Progress = 0;
                ToolsReady = operationKind == "toolInstall" ? _toolsReadyBeforeOperation : true;
                CanInstallTools = operationKind == "toolInstall";
                SetupRequired = operationKind == "toolInstall" && !ToolsReady;
                StatusText = operationKind == "toolInstall"
                    ? "Tool installation cancelled."
                    : "Download cancelled.";
                break;
            case "operationFailed":
                FinishOperation();
                var error = message.Data.GetProperty("error");
                StatusText = error.GetProperty("message").GetString() ?? "The operation failed.";
                DetailsText = error.TryGetProperty("details", out var details) && details.ValueKind == JsonValueKind.String
                    ? details.GetString() ?? string.Empty
                    : string.Empty;
                Failed = true;
                CanInstallTools = operationKind == "toolInstall";
                ToolsReady = operationKind == "toolInstall" ? _toolsReadyBeforeOperation : ToolsReady;
                SetupRequired = operationKind == "toolInstall" && !ToolsReady;
                break;
        }

        RefreshComputedProperties();
    }

    private void OnBackendExited(string? message) => Dispatcher.UIThread.Post(() =>
    {
        _activeOperationId = null;
        ApplyFailure("The download engine stopped unexpectedly.", message ?? "The backend exited.");
        ToolsReady = false;
        EngineUnavailable = true;
    });

    private void SetWorking(string status, bool cancellable)
    {
        Busy = true;
        Cancellable = cancellable;
        Failed = false;
        ShowDetails = false;
        DetailsText = string.Empty;
        StatusText = status;
        RefreshComputedProperties();
    }

    private void FinishOperation()
    {
        _activeOperationId = null;
        Busy = false;
        Cancellable = false;
        Failed = false;
    }

    private void ApplyFailure(string status, object error)
    {
        Busy = false;
        Cancellable = false;
        Failed = true;
        StatusText = error is BackendRequestException requestError && requestError.Error.Code == "invalidUrl"
            ? requestError.Error.Message
            : status;
        DetailsText = error switch
        {
            BackendRequestException backend => backend.Error.Details ?? backend.Error.Message,
            Exception exception => exception.Message,
            _ => error.ToString() ?? string.Empty,
        };
        RefreshComputedProperties();
    }

    partial void OnUrlChanged(string value) => RefreshComputedProperties();
    partial void OnOutputFolderChanged(string value) => RefreshComputedProperties();
    partial void OnAudioOnlyChanged(bool value) => RefreshComputedProperties();
    partial void OnBusyChanged(bool value) => RefreshComputedProperties();
    partial void OnCancellableChanged(bool value) => RefreshComputedProperties();
    partial void OnToolsReadyChanged(bool value) => RefreshComputedProperties();
    partial void OnSetupRequiredChanged(bool value) => RefreshComputedProperties();
    partial void OnUpdateAvailableChanged(bool value) => RefreshComputedProperties();
    partial void OnCompletedChanged(bool value) => RefreshComputedProperties();
    partial void OnCompletedPathChanged(string? value) => RefreshComputedProperties();
    partial void OnDetailsTextChanged(string value) => RefreshComputedProperties();

    partial void OnShowDetailsChanged(bool value) => OnPropertyChanged(nameof(DetailsButtonText));
    partial void OnProgressChanged(double value) => OnPropertyChanged(nameof(IsProgressIndeterminate));

    partial void OnEngineUnavailableChanged(bool value) => OnPropertyChanged(nameof(ShowRestartButton));

    private void RefreshComputedProperties()
    {
        OnPropertyChanged(nameof(CanEdit));
        OnPropertyChanged(nameof(CanBrowse));
        OnPropertyChanged(nameof(CanChangeQuality));
        OnPropertyChanged(nameof(CanDownload));
        OnPropertyChanged(nameof(ShowDownloadButton));
        OnPropertyChanged(nameof(ShowCancelButton));
        OnPropertyChanged(nameof(ShowInstallPanel));
        OnPropertyChanged(nameof(ShowUpdatePanel));
        OnPropertyChanged(nameof(ShowDetailsButton));
        OnPropertyChanged(nameof(IsProgressIndeterminate));
        OnPropertyChanged(nameof(HasCompletedFile));
        OnPropertyChanged(nameof(DetailsButtonText));
        OnPropertyChanged(nameof(ShowRestartButton));
    }
}
