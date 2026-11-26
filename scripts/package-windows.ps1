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
    $guiOut = Join-Path $stageDir "gui-publish"
    $packageDir = Join-Path $stageDir "package"
    $distDir = Join-Path $root "dist"

    New-Item -ItemType Directory -Path $dotnetOut -Force | Out-Null
    New-Item -ItemType Directory -Path $guiOut -Force | Out-Null
    New-Item -ItemType Directory -Path $packageDir -Force | Out-Null
    New-Item -ItemType Directory -Path $distDir -Force | Out-Null

    Write-Host "==> 打包版本：$packageVersion（git: $gitCommit）"
    Write-Host "==> 发布 ffr-export.exe（win-x64 self-contained single-file，不向 TEMP 自解压原生库）"

    $csprojPath = Join-Path $root "src\FfrExport\FfrExport.csproj"
    & $dotnetBin publish $csprojPath `
        -c Release `
        -r win-x64 `
        --self-contained true `
        -p:PublishSingleFile=true `
        -p:IncludeNativeLibrariesForSelfExtract=false `
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

    Write-Host "==> 发布 ffr-export-gui.exe（win-x64 self-contained single-file WinForms，不向 TEMP 自解压原生库）"
    $guiCsprojPath = Join-Path $root "src\FfrExport.Gui\FfrExport.Gui.csproj"
    & $dotnetBin publish $guiCsprojPath `
        -c Release `
        -r win-x64 `
        --self-contained true `
        -p:PublishSingleFile=true `
        -p:IncludeNativeLibrariesForSelfExtract=false `
        -p:EnableCompressionInSingleFile=true `
        -p:DebugType=None `
        -p:DebugSymbols=false `
        -p:AssemblyName=ffr-export-gui `
        "-p:Version=$packageVersion" `
        "-p:FfrGitCommit=$gitCommit" `
        -o $guiOut
    if ($LASTEXITCODE -ne 0) {
        throw "错误：dotnet publish 发布 $guiCsprojPath 失败（退出码 $LASTEXITCODE）"
    }

    $ffrExportGuiExe = Join-Path $guiOut "ffr-export-gui.exe"
    if (-not (Test-Path -LiteralPath $ffrExportGuiExe -PathType Leaf)) {
        throw "错误：dotnet publish 未生成 $ffrExportGuiExe"
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
    Copy-Item -LiteralPath $ffrExportGuiExe -Destination (Join-Path $packageDir "ffr-export-gui.exe") -Force
    Copy-Item -LiteralPath $ffrDbExe -Destination (Join-Path $packageDir "ffr-db.exe") -Force
    Copy-Item -LiteralPath (Join-Path $root "known-builds.json") -Destination (Join-Path $packageDir "known-builds.json") -Force
    Copy-Item -LiteralPath (Join-Path $root "README.md") -Destination (Join-Path $packageDir "README.md") -Force
    Copy-Item -LiteralPath (Join-Path $root "LICENSE") -Destination (Join-Path $packageDir "LICENSE") -Force

    # Copy any native sidecar DLLs emitted alongside the single-file executables and verify duplicates match.
    $copiedNativeDlls = [System.Collections.Generic.List[string]]::new()
    foreach ($publishDir in @($dotnetOut, $guiOut)) {
        $dlls = @(Get-ChildItem -LiteralPath $publishDir -File -Filter "*.dll")
        foreach ($dll in $dlls) {
            $dstPath = Join-Path $packageDir $dll.Name
            if (Test-Path -LiteralPath $dstPath -PathType Leaf) {
                $existingHash = (Get-FileHash -LiteralPath $dstPath -Algorithm SHA256).Hash
                $incomingHash = (Get-FileHash -LiteralPath $dll.FullName -Algorithm SHA256).Hash
                if ($existingHash -ne $incomingHash) {
                    throw "错误：CLI 与 GUI 产出的同名原生库内容不一致：$($dll.Name)"
                }
            } else {
                Copy-Item -LiteralPath $dll.FullName -Destination $dstPath -Force
                $copiedNativeDlls.Add($dll.Name)
            }
        }
    }
    if ($copiedNativeDlls.Count -gt 0) {
        Write-Host "==> 已随包复制原生库（$($copiedNativeDlls.Count) 个）：$($copiedNativeDlls -join ', ')"
    } else {
        Write-Host "==> 本次 .NET 10 win-x64 单文件发布未产出额外外部原生 DLL（运行时原生代码已内置于 exe 中，不向 TEMP 解压）"
    }

    # Self-test packaged executables with TEMP / TMP pointed to an icacls deny-write directory.
    Write-Host "==> 自检打包目录内的 exe（icacls 拒绝写入 TEMP / TMP 模拟）"
    $selfTestStderr = Join-Path $stageDir "gui-selftest-stderr.log"
    $cliHelpStdout = Join-Path $stageDir "cli-help-stdout.log"
    $cliHelpStderr = Join-Path $stageDir "cli-help-stderr.log"
    $dbHelpStdout = Join-Path $stageDir "db-help-stdout.log"
    $dbHelpStderr = Join-Path $stageDir "db-help-stderr.log"
    $deniedTempDir = Join-Path $stageDir "denied-temp"
    New-Item -ItemType Directory -Path $deniedTempDir -Force | Out-Null

    $previousAgpHome = $env:AGP_HOME
    $hadAgpHome = Test-Path Env:AGP_HOME
    $previousTemp = $env:TEMP
    $hadTemp = Test-Path Env:TEMP
    $previousTmp = $env:TMP
    $hadTmp = Test-Path Env:TMP

    $selfTestAgpHome = Join-Path $stageDir "agp-home"
    New-Item -ItemType Directory -Path $selfTestAgpHome -Force | Out-Null
    try {
        & icacls $deniedTempDir /deny '*S-1-1-0:(OI)(CI)(W)' | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw "错误：使用 icacls 为临时目录设置拒绝写入 ACL 失败：$deniedTempDir（退出码 $LASTEXITCODE）"
        }
        $aclBlocked = $false
        try {
            [System.IO.File]::WriteAllText((Join-Path $deniedTempDir "acl-probe.tmp"), "blocked")
        } catch [System.UnauthorizedAccessException] {
            $aclBlocked = $true
        }
        if (-not $aclBlocked) {
            throw "错误：icacls 拒绝写入 ACL 未生效，仍可向目录写入文件：$deniedTempDir"
        }

        $env:AGP_HOME = $selfTestAgpHome
        $env:TEMP = $deniedTempDir
        $env:TMP = $deniedTempDir

        $packagedGuiExe = Join-Path $packageDir "ffr-export-gui.exe"
        $packagedCliExe = Join-Path $packageDir "ffr-export.exe"
        $packagedDbExe = Join-Path $packageDir "ffr-db.exe"

        # WinExe (GUI subsystem): wait explicitly with Start-Process and capture stderr on failure.
        $selfTest = Start-Process -FilePath $packagedGuiExe -ArgumentList @('--self-test') -Wait -PassThru -NoNewWindow -RedirectStandardError $selfTestStderr
        if ($selfTest.ExitCode -ne 0) {
            if (Test-Path -LiteralPath $selfTestStderr -PathType Leaf) {
                $stderrText = Get-Content -LiteralPath $selfTestStderr -Raw -Encoding UTF8
                if (-not [string]::IsNullOrWhiteSpace($stderrText)) {
                    [Console]::Error.WriteLine($stderrText)
                }
            }
            throw "错误：TEMP 被 icacls 拒绝写入时 ffr-export-gui.exe --self-test 自检失败（退出码 $($selfTest.ExitCode)）"
        }

        $cliHelp = Start-Process -FilePath $packagedCliExe -ArgumentList @('--help') -Wait -PassThru -NoNewWindow -RedirectStandardOutput $cliHelpStdout -RedirectStandardError $cliHelpStderr
        if ($cliHelp.ExitCode -ne 0) {
            if (Test-Path -LiteralPath $cliHelpStderr -PathType Leaf) {
                [Console]::Error.WriteLine((Get-Content -LiteralPath $cliHelpStderr -Raw -Encoding UTF8))
            }
            throw "错误：TEMP 被 icacls 拒绝写入时 ffr-export.exe --help 失败（退出码 $($cliHelp.ExitCode)）"
        }

        $dbHelp = Start-Process -FilePath $packagedDbExe -ArgumentList @('--help') -Wait -PassThru -NoNewWindow -RedirectStandardOutput $dbHelpStdout -RedirectStandardError $dbHelpStderr
        if ($dbHelp.ExitCode -ne 0) {
            if (Test-Path -LiteralPath $dbHelpStderr -PathType Leaf) {
                [Console]::Error.WriteLine((Get-Content -LiteralPath $dbHelpStderr -Raw -Encoding UTF8))
            }
            throw "错误：TEMP 被 icacls 拒绝写入时 ffr-db.exe --help 失败（退出码 $($dbHelp.ExitCode)）"
        }

        $leakedTempFiles = @(Get-ChildItem -LiteralPath $deniedTempDir -Force)
        if ($leakedTempFiles.Count -ne 0) {
            throw "错误：拒绝写入的 TEMP 目录内出现了意外文件：$($leakedTempFiles.Name -join ', ')"
        }
    } finally {
        if ($hadAgpHome) { $env:AGP_HOME = $previousAgpHome } else { Remove-Item Env:AGP_HOME -ErrorAction SilentlyContinue }
        if ($hadTemp) { $env:TEMP = $previousTemp } else { Remove-Item Env:TEMP -ErrorAction SilentlyContinue }
        if ($hadTmp) { $env:TMP = $previousTmp } else { Remove-Item Env:TMP -ErrorAction SilentlyContinue }
        if (Test-Path -LiteralPath $deniedTempDir) {
            & icacls $deniedTempDir /remove:d '*S-1-1-0' 2>$null | Out-Null
        }
        foreach ($tmpLog in @($selfTestStderr, $cliHelpStdout, $cliHelpStderr, $dbHelpStdout, $dbHelpStderr)) {
            if (Test-Path -LiteralPath $tmpLog) {
                Remove-Item -LiteralPath $tmpLog -Force -ErrorAction SilentlyContinue
            }
        }
    }

    # Write a concise Chinese quick-start guide (<= 10 lines, UTF-8 without BOM, CRLF for Windows Notepad).
    $utf8NoBom = [System.Text.UTF8Encoding]::new($false)
    $usageLines = @(
        '《FINAL FANTASY RESONANCE》数据导出工具使用说明',
        '',
        '1. 将本压缩包完整解压到同一个文件夹（不要单独移动某个 exe 或原生库）。',
        '2. 如双击时提示低完整性级别，可右键压缩包 -> 属性 -> 解除锁定，再用资源管理器重新解压。',
        '3. 双击打开 ffr-export-gui.exe，可一键获取 .usmap 或手动选择，确认后点「开始导出」。',
        '4. 导出完成后可点「打开输出文件夹」查看 ffr.sqlite，或点「复制哈希」核对版本。',
        '5. 如需清理游戏目录中的 UE4SS，可点击主窗口「清理 UE4SS…」按钮；命令行用法见 README.md。'
    )
    $usagePath = Join-Path $packageDir "使用说明.txt"
    [System.IO.File]::WriteAllText($usagePath, (($usageLines -join "`r`n") + "`r`n"), $utf8NoBom)

    # Write inner SHA256SUMS covering every packaged file (sorted with Ordinal order, LF endings).
    $innerFiles = @(
        Get-ChildItem -LiteralPath $packageDir -File |
            Where-Object { $_.Name -ne "SHA256SUMS" } |
            ForEach-Object { $_.Name } |
            Sort-Object { $_ } -CaseSensitive
    )
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
    $zipEntries = @(
        Get-ChildItem -LiteralPath $packageDir -File |
            ForEach-Object { $_.Name } |
            Sort-Object { $_ } -CaseSensitive
    )
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
    Write-Host "==> 压缩包内文件列表："
    $verifyZip = [System.IO.Compression.ZipFile]::OpenRead($zipPath)
    try {
        $verifyZip.Entries | Select-Object FullName, Length, CompressedLength | Format-Table -AutoSize
    } finally {
        $verifyZip.Dispose()
    }
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
