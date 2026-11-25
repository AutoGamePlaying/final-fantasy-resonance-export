[CmdletBinding()]
param(
    [string]$Version = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $root

$stageDir = $null
try {
    # Disable telemetry, logos, and persistent MSBuild / compiler servers.
    $env:DOTNET_CLI_TELEMETRY_OPTOUT = "1"
    $env:DOTNET_NOLOGO = "1"
    $env:MSBUILDDISABLENODEREUSE = "1"
    $env:DOTNET_CLI_USE_MSBUILD_SERVER = "0"
    $env:UseSharedCompilation = "false"

    # Resolve dotnet executable: DOTNET_ROOT -> .deps/dotnet -> PATH.
    $dotnetBin = $null
    if (-not [string]::IsNullOrWhiteSpace($env:DOTNET_ROOT)) {
        $candidate = Join-Path $env:DOTNET_ROOT "dotnet.exe"
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            $dotnetBin = $candidate
        }
    }
    if (-not $dotnetBin) {
        $localDotnet = Join-Path $root ".deps\dotnet\dotnet.exe"
        if (Test-Path -LiteralPath $localDotnet -PathType Leaf) {
            $env:DOTNET_ROOT = Join-Path $root ".deps\dotnet"
            $dotnetBin = $localDotnet
        }
    }
    if (-not $dotnetBin) {
        $dotnetCmd = Get-Command dotnet -ErrorAction SilentlyContinue
        if ($dotnetCmd) {
            $dotnetBin = $dotnetCmd.Source
        }
    }
    if (-not $dotnetBin) {
        throw "错误：找不到 dotnet 可执行文件（请安装 .NET 10 SDK 或设置 DOTNET_ROOT）"
    }

    # Resolve cargo executable: prefer PATH, then CARGO_HOME / USERPROFILE\.cargo\bin.
    $cargoBin = $null
    $cargoCmd = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cargoCmd) {
        $cargoBin = $cargoCmd.Source
    }
    if (-not $cargoBin -and -not [string]::IsNullOrWhiteSpace($env:CARGO_HOME)) {
        $candidate = Join-Path $env:CARGO_HOME "bin\cargo.exe"
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            $cargoBin = $candidate
        }
    }
    if (-not $cargoBin -and -not [string]::IsNullOrWhiteSpace($env:USERPROFILE)) {
        $candidate = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            $cargoBin = $candidate
        }
    }
    if (-not $cargoBin) {
        throw "错误：找不到 cargo 可执行文件（请确认 PATH 或 Rust 工具链已安装）"
    }

    # Resolve package version: explicit -Version -> exact git tag on HEAD -> VERSION file.
    $packageVersion = $Version.Trim()
    if ($packageVersion.StartsWith("v", [System.StringComparison]::Ordinal)) {
        $packageVersion = $packageVersion.Substring(1)
    }
    if ([string]::IsNullOrWhiteSpace($packageVersion)) {
        $rawTag = ""
        try {
            $tagOutput = & git describe --tags --exact-match HEAD 2>$null
            if ($LASTEXITCODE -eq 0 -and $tagOutput) {
                $rawTag = ($tagOutput | Select-Object -First 1).Trim()
            }
        } catch {
            $rawTag = ""
        }

        if (-not [string]::IsNullOrWhiteSpace($rawTag)) {
            $packageVersion = if ($rawTag.StartsWith("v", [System.StringComparison]::Ordinal)) {
                $rawTag.Substring(1)
            } else {
                $rawTag
            }
        } else {
            $versionFile = Join-Path $root "VERSION"
            if (-not (Test-Path -LiteralPath $versionFile -PathType Leaf)) {
                throw "错误：找不到 git tag 且仓库根目录缺少 VERSION 文件：$versionFile"
            }
            $packageVersion = (Get-Content -LiteralPath $versionFile -Raw -Encoding UTF8).Trim()
        }
    }
    if ([string]::IsNullOrWhiteSpace($packageVersion)) {
        throw "错误：解析到的版本号为空"
    }

    # Resolve short git commit hash for embedding into assembly metadata.
    $gitCommit = "unknown"
    try {
        $commitOutput = & git rev-parse --short=12 HEAD 2>$null
        if ($LASTEXITCODE -eq 0 -and $commitOutput) {
            $trimmedCommit = ($commitOutput | Select-Object -First 1).Trim()
            if (-not [string]::IsNullOrWhiteSpace($trimmedCommit)) {
                $gitCommit = $trimmedCommit
            }
        }
    } catch {
        $gitCommit = "unknown"
    }

    $stageDir = Join-Path ([System.IO.Path]::GetTempPath()) ("ffr-win-pkg-" + [System.Guid]::NewGuid().ToString("N"))
    $dotnetOut = Join-Path $stageDir "dotnet-publish"
    $packageDir = Join-Path $stageDir "package"
    $distDir = Join-Path $root "dist"

    New-Item -ItemType Directory -Path $dotnetOut -Force | Out-Null
    New-Item -ItemType Directory -Path $packageDir -Force | Out-Null
    New-Item -ItemType Directory -Path $distDir -Force | Out-Null

    Write-Host "==> 打包版本：$packageVersion（git: $gitCommit）"
    Write-Host "==> 发布 ffr-export.exe（win-x64 self-contained single-file）"

    $csprojPath = Join-Path $root "src\FfrExport\FfrExport.csproj"
    & $dotnetBin publish $csprojPath `
        -c Release `
        -r win-x64 `
        --self-contained true `
        -p:PublishSingleFile=true `
        -p:IncludeNativeLibrariesForSelfExtract=true `
        -p:EnableCompressionInSingleFile=true `
        -p:DebugType=None `
        -p:DebugSymbols=false `
        -p:AssemblyName=ffr-export `
        "-p:Version=$packageVersion" `
        "-p:FfrGitCommit=$gitCommit" `
        -o $dotnetOut
    if ($LASTEXITCODE -ne 0) {
        throw "错误：dotnet publish 发布 $csprojPath 失败（退出码 $LASTEXITCODE）"
    }

    $ffrExportExe = Join-Path $dotnetOut "ffr-export.exe"
    if (-not (Test-Path -LiteralPath $ffrExportExe -PathType Leaf)) {
        throw "错误：dotnet publish 未生成 $ffrExportExe"
    }

    Write-Host "==> 原生编译 ffr-db.exe（cargo build --release -p ffr-db）"
    & $cargoBin build --release -p ffr-db
    if ($LASTEXITCODE -ne 0) {
        throw "错误：cargo build --release -p ffr-db 失败（退出码 $LASTEXITCODE）"
    }

    $ffrDbExe = Join-Path $root "target\release\ffr-db.exe"
    if (-not (Test-Path -LiteralPath $ffrDbExe -PathType Leaf)) {
        throw "错误：cargo build 未生成 $ffrDbExe"
    }

    foreach ($required in @("known-builds.json", "README.md", "LICENSE")) {
        $reqPath = Join-Path $root $required
        if (-not (Test-Path -LiteralPath $reqPath -PathType Leaf)) {
            throw "错误：打包缺少必需文件：$reqPath"
        }
    }

    Copy-Item -LiteralPath $ffrExportExe -Destination (Join-Path $packageDir "ffr-export.exe") -Force
    Copy-Item -LiteralPath $ffrDbExe -Destination (Join-Path $packageDir "ffr-db.exe") -Force
    Copy-Item -LiteralPath (Join-Path $root "known-builds.json") -Destination (Join-Path $packageDir "known-builds.json") -Force
    Copy-Item -LiteralPath (Join-Path $root "README.md") -Destination (Join-Path $packageDir "README.md") -Force
    Copy-Item -LiteralPath (Join-Path $root "LICENSE") -Destination (Join-Path $packageDir "LICENSE") -Force

    # Write inner SHA256SUMS matching package-windows.sh layout (UTF-8 without BOM, LF endings).
    $utf8NoBom = [System.Text.UTF8Encoding]::new($false)
    $innerFiles = @("LICENSE", "README.md", "ffr-db.exe", "ffr-export.exe", "known-builds.json")
    $innerLines = foreach ($name in $innerFiles) {
        $filePath = Join-Path $packageDir $name
        $digest = (Get-FileHash -LiteralPath $filePath -Algorithm SHA256).Hash.ToLowerInvariant()
        "$digest  $name"
    }
    $innerSumsPath = Join-Path $packageDir "SHA256SUMS"
    [System.IO.File]::WriteAllText($innerSumsPath, (($innerLines -join "`n") + "`n"), $utf8NoBom)

    $zipName = "ffr-export-$packageVersion-win-x64.zip"
    $zipPath = Join-Path $distDir $zipName
    if (Test-Path -LiteralPath $zipPath) {
        Remove-Item -LiteralPath $zipPath -Force
    }

    Write-Host "==> 生成压缩包：$zipPath"
    $zipEntries = @("LICENSE", "README.md", "SHA256SUMS", "ffr-db.exe", "ffr-export.exe", "known-builds.json")
    $archive = [System.IO.Compression.ZipFile]::Open($zipPath, [System.IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($name in $zipEntries) {
            $sourceFile = Join-Path $packageDir $name
            [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                $archive,
                $sourceFile,
                $name,
                [System.IO.Compression.CompressionLevel]::Optimal
            ) | Out-Null
        }
    } finally {
        $archive.Dispose()
    }

    # Copy known-builds.json and generate release-level dist/SHA256SUMS for GitHub Releases upload.
    $distKnownBuilds = Join-Path $distDir "known-builds.json"
    Copy-Item -LiteralPath (Join-Path $root "known-builds.json") -Destination $distKnownBuilds -Force

    $releaseFiles = @($zipName, "known-builds.json")
    $releaseLines = foreach ($name in $releaseFiles) {
        $filePath = Join-Path $distDir $name
        $digest = (Get-FileHash -LiteralPath $filePath -Algorithm SHA256).Hash.ToLowerInvariant()
        "$digest  $name"
    }
    $distSumsPath = Join-Path $distDir "SHA256SUMS"
    [System.IO.File]::WriteAllText($distSumsPath, (($releaseLines -join "`n") + "`n"), $utf8NoBom)

    Write-Host "==> 打包完成：$zipPath"
    Get-Item -LiteralPath $zipPath, $distSumsPath, $distKnownBuilds | Format-Table Name, Length -AutoSize
    Write-Host "==> 压缩包内文件与校验（zip 内 SHA256SUMS）："
    Write-Host ($innerLines -join "`n")
    Write-Host "==> 发布产物校验（dist/SHA256SUMS）："
    Write-Host ($releaseLines -join "`n")
} finally {
    Pop-Location
    if ($stageDir -and (Test-Path -LiteralPath $stageDir)) {
        Remove-Item -LiteralPath $stageDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
