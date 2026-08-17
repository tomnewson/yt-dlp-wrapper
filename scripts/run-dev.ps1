$ErrorActionPreference = "Stop"

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$project = Join-Path $repositoryRoot "src/dotnet/YtDlpWrapper.App/YtDlpWrapper.App.csproj"
$backend = Join-Path $repositoryRoot "target/x86_64-pc-windows-msvc/debug/yt-dlp-wrapper-backend.exe"
$appOutput = Join-Path $repositoryRoot "src/dotnet/YtDlpWrapper.App/bin/Debug/net10.0"

Push-Location $repositoryRoot
try {
    & cargo build --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) {
        throw "The Rust backend build failed with exit code $LASTEXITCODE."
    }

    & dotnet build $project
    if ($LASTEXITCODE -ne 0) {
        throw "The .NET frontend build failed with exit code $LASTEXITCODE."
    }

    Copy-Item -LiteralPath $backend -Destination $appOutput -Force

    & dotnet run --project $project --no-build
    if ($LASTEXITCODE -ne 0) {
        throw "The application exited with code $LASTEXITCODE."
    }
}
finally {
    Pop-Location
}
