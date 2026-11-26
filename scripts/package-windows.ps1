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
        throw "Error: dotnet executable not found (install .NET 10 SDK or set DOTNET_ROOT)."
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
        throw "Error: cargo executable not found (ensure the Rust toolchain is installed and on PATH)."
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
                throw "Error: no git tag on HEAD and VERSION file is missing: $versionFile"
            }
            $packageVersion = (Get-Content -LiteralPath $versionFile -Raw -Encoding UTF8).Trim()
        }
    }
    if ([string]::IsNullOrWhiteSpace($packageVersion)) {
        throw "Error: resolved package version is empty."
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

    Write-Host "==> Packaging version: $packageVersion (git: $gitCommit)"
    Write-Host "==> Publishing ffr-export.exe (win-x64 self-contained single-file, no TEMP native self-extract)"

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
        throw "Error: dotnet publish failed for $csprojPath (exit code $LASTEXITCODE)"
    }

    $ffrExportExe = Join-Path $dotnetOut "ffr-export.exe"
    if (-not (Test-Path -LiteralPath $ffrExportExe -PathType Leaf)) {
        throw "Error: dotnet publish did not produce $ffrExportExe"
    }

    Write-Host "==> Publishing ffr-export-gui.exe (win-x64 self-contained single-file WinForms, no TEMP native self-extract)"
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
        throw "Error: dotnet publish failed for $guiCsprojPath (exit code $LASTEXITCODE)"
    }

    $ffrExportGuiExe = Join-Path $guiOut "ffr-export-gui.exe"
    if (-not (Test-Path -LiteralPath $ffrExportGuiExe -PathType Leaf)) {
        throw "Error: dotnet publish did not produce $ffrExportGuiExe"
    }

    Write-Host "==> Building native ffr-db.exe (cargo build --release -p ffr-db)"
    & $cargoBin build --release -p ffr-db
    if ($LASTEXITCODE -ne 0) {
        throw "Error: cargo build --release -p ffr-db failed (exit code $LASTEXITCODE)"
    }

    $ffrDbExe = Join-Path $root "target\release\ffr-db.exe"
    if (-not (Test-Path -LiteralPath $ffrDbExe -PathType Leaf)) {
        throw "Error: cargo build did not produce $ffrDbExe"
    }

    foreach ($required in @("known-builds.json", "README.md", "LICENSE")) {
        $reqPath = Join-Path $root $required
        if (-not (Test-Path -LiteralPath $reqPath -PathType Leaf)) {
            throw "Error: required packaging file not found: $reqPath"
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
                    throw "Error: mismatched sidecar native DLL between CLI and GUI publish outputs: $($dll.Name)"
                }
            } else {
                Copy-Item -LiteralPath $dll.FullName -Destination $dstPath -Force
                $copiedNativeDlls.Add($dll.Name)
            }
        }
    }
    if ($copiedNativeDlls.Count -gt 0) {
        Write-Host "==> Copied $($copiedNativeDlls.Count) sidecar native DLL(s): $($copiedNativeDlls -join ', ')"
    } else {
        Write-Host "==> No external native sidecar DLLs emitted by .NET 10 win-x64 single-file publish"
    }

    # Self-test packaged executables with TEMP / TMP pointed to an icacls deny-write directory.
    Write-Host "==> Self-testing packaged executables (icacls deny-write TEMP / TMP simulation)"
    $selfTestStderr = Join-Path $stageDir "gui-selftest-stderr.log"
    $cliHelpStdout = Join-Path $stageDir "cli-help-stdout.log"
    $cliHelpStderr = Join-Path $stageDir "cli-help-stderr.log"
    $dbHelpStdout = Join-Path $stageDir "db-help-stdout.log"
    $dbHelpStderr = Join-Path $stageDir "db-help-stderr.log"
    $deniedTempDir = Join-Path $stageDir "denied-temp"
    New-Item -ItemType Directory -Path $deniedTempDir -Force | Out-Null

    $previousAgpHome = $env:AGP_HOME
    $hadAgpHome = Test-Path Env:AGP_HOME
    $previousAgpLang = $env:AGP_LANG
    $hadAgpLang = Test-Path Env:AGP_LANG
    $previousTemp = $env:TEMP
    $hadTemp = Test-Path Env:TEMP
    $previousTmp = $env:TMP
    $hadTmp = Test-Path Env:TMP

    $selfTestAgpHome = Join-Path $stageDir "agp-home"
    New-Item -ItemType Directory -Path $selfTestAgpHome -Force | Out-Null
    try {
        & icacls $deniedTempDir /deny '*S-1-1-0:(OI)(CI)(WD,AD,WEA,WA)' | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw "Error: icacls failed to apply deny-write ACL on temp directory: $deniedTempDir (exit code $LASTEXITCODE)"
        }
        $aclBlocked = $false
        try {
            [System.IO.File]::WriteAllText((Join-Path $deniedTempDir "acl-probe.tmp"), "blocked")
        } catch [System.UnauthorizedAccessException] {
            $aclBlocked = $true
        }
        if (-not $aclBlocked) {
            throw "Error: icacls deny-write ACL did not take effect; file write still succeeded in $deniedTempDir"
        }

        $env:AGP_HOME = $selfTestAgpHome
        Remove-Item Env:AGP_LANG -ErrorAction SilentlyContinue
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
            throw "Error: ffr-export-gui.exe --self-test failed with icacls deny-write TEMP (exit code $($selfTest.ExitCode))"
        }

        # Verify CLI --help works in all three bundled single-file languages (en, zh-Hans, ja).
        $langChecks = @(
            @{ Code = 'en'; Expected = 'Usage:' },
            @{ Code = 'zh-Hans'; Expected = '用法' },
            @{ Code = 'ja'; Expected = '使い方:' }
        )
        foreach ($check in $langChecks) {
            $cliArgs = if ($check.Code -eq 'en') { @('--help') } else { @('--lang', $check.Code, '--help') }
            $cliHelp = Start-Process -FilePath $packagedCliExe -ArgumentList $cliArgs -Wait -PassThru -NoNewWindow -RedirectStandardOutput $cliHelpStdout -RedirectStandardError $cliHelpStderr
            if ($cliHelp.ExitCode -ne 0) {
                if (Test-Path -LiteralPath $cliHelpStderr -PathType Leaf) {
                    [Console]::Error.WriteLine((Get-Content -LiteralPath $cliHelpStderr -Raw -Encoding UTF8))
                }
                throw "Error: ffr-export.exe ($($check.Code)) --help failed with icacls deny-write TEMP (exit code $($cliHelp.ExitCode))"
            }
            $helpOut = Get-Content -LiteralPath $cliHelpStdout -Raw -Encoding UTF8
            if (-not $helpOut.Contains($check.Expected)) {
                throw "Error: ffr-export.exe ($($check.Code)) --help output did not contain expected localized marker."
            }
        }

        $dbHelp = Start-Process -FilePath $packagedDbExe -ArgumentList @('--help') -Wait -PassThru -NoNewWindow -RedirectStandardOutput $dbHelpStdout -RedirectStandardError $dbHelpStderr
        if ($dbHelp.ExitCode -ne 0) {
            if (Test-Path -LiteralPath $dbHelpStderr -PathType Leaf) {
                [Console]::Error.WriteLine((Get-Content -LiteralPath $dbHelpStderr -Raw -Encoding UTF8))
            }
            throw "Error: ffr-db.exe --help failed with icacls deny-write TEMP (exit code $($dbHelp.ExitCode))"
        }

        # Lift the deny ACE before inspecting the directory; the check itself must not be blocked by it.
        & icacls $deniedTempDir /remove:d '*S-1-1-0' | Out-Null
        $leakedTempFiles = @(Get-ChildItem -LiteralPath $deniedTempDir -Force)
        if ($leakedTempFiles.Count -ne 0) {
            throw "Error: unexpected files appeared inside deny-write TEMP directory: $($leakedTempFiles.Name -join ', ')"
        }
    } finally {
        if ($hadAgpHome) { $env:AGP_HOME = $previousAgpHome } else { Remove-Item Env:AGP_HOME -ErrorAction SilentlyContinue }
        if ($hadAgpLang) { $env:AGP_LANG = $previousAgpLang } else { Remove-Item Env:AGP_LANG -ErrorAction SilentlyContinue }
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

    # Write Windows quick-start guide (English primary with concise Chinese & Japanese sections, UTF-8 without BOM, CRLF for Notepad).
    $utf8NoBom = [System.Text.UTF8Encoding]::new($false)
    $usageLines = @(
        'FINAL FANTASY RESONANCE Data Exporter (Windows Quick Start)',
        '',
        '1. Extract this entire zip archive into one folder (keep all .exe files and known-builds.json together).',
        '2. If Windows warns about Low Integrity Level when launching, right-click the .zip -> Properties -> Unblock, then re-extract with File Explorer.',
        '3. Double-click ffr-export-gui.exe. Switch language anytime in the top-right dropdown (English / 简体中文 / 日本語).',
        '4. Click "Fetch .usmap (launches game once)" or select an existing .usmap file, then click "Start Export".',
        '5. When finished, click "Open Output Folder" to view ffr.sqlite or "Copy Hashes" to compare against known-builds.json.',
        '6. To remove UE4SS from the game folder, click "Clean up UE4SS..." in the main window. For CLI usage, see README.md.',
        '',
        '---',
        '[简体中文简版]',
        '1. 将整个压缩包解压到同一个文件夹（保留所有 exe 与 known-builds.json 在一起）。',
        '2. 若启动时提示低完整性级别，右键压缩包 -> 属性 -> 解除锁定，再用资源管理器重新解压。',
        '3. 双击 ffr-export-gui.exe，可在右上角切换语言（English / 简体中文 / 日本語）；获取或选择 .usmap 后点「开始导出」。',
        '4. 完成后可点「打开输出文件夹」查看 ffr.sqlite，或点「清理 UE4SS…」卸载 UE4SS；命令行用法见 README.md。',
        '',
        '---',
        '[日本語クイックガイド]',
        '1. zip 全体を 1 つのフォルダに展開してください（すべての exe と known-builds.json を同じ場所に保ちます）。',
        '2. 起動時に低整合性レベルの警告が出た場合は、zip を右クリック -> プロパティ -> 許可する（ブロック解除）後にエクスプローラーで再展開してください。',
        '3. ffr-export-gui.exe を開き、右上で表示言語（English / 简体中文 / 日本語）を切り替えられます。.usmap を取得または選択して「エクスポート開始」をクリックします。',
        '4. 完了後は「出力フォルダを開く」で ffr.sqlite を確認できます。UE4SS の削除は「UE4SS をクリーンアップ…」から行えます。CLI の使い方は README.md を参照してください。'
    )
    $usagePath = Join-Path $packageDir "README-Windows.txt"
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

    Write-Host "==> Creating archive: $zipPath"
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

    Write-Host "==> Packaging complete: $zipPath"
    Get-Item -LiteralPath $zipPath, $distSumsPath, $distKnownBuilds | Format-Table Name, Length -AutoSize
    Write-Host "==> Zip archive entries:"
    $verifyZip = [System.IO.Compression.ZipFile]::OpenRead($zipPath)
    try {
        $verifyZip.Entries | Select-Object FullName, Length, CompressedLength | Format-Table -AutoSize
    } finally {
        $verifyZip.Dispose()
    }
    Write-Host "==> Packaged file checksums (inner SHA256SUMS):"
    Write-Host ($innerLines -join "`n")
    Write-Host "==> Release artifact checksums (dist/SHA256SUMS):"
    Write-Host ($releaseLines -join "`n")
} finally {
    Pop-Location
    if ($stageDir -and (Test-Path -LiteralPath $stageDir)) {
        Remove-Item -LiteralPath $stageDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
