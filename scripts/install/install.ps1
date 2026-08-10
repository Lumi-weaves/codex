[CmdletBinding()]
param(
    [string]$Release = $env:LUMI_RELEASE,
    [string]$Target = $env:LUMI_TARGET
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

if ([string]::IsNullOrWhiteSpace($Release)) {
    $Release = "latest"
}

$ReleasesApiBase = "https://api.github.com/repos/Lumi-weaves/codex"
$ReleasesDownloadBase = "https://github.com/Lumi-weaves/codex/releases/download"
$ReleasesAssetTimeoutSec = 300
$TargetAllowlist = @(
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc"
)

function Write-Step {
    param(
        [string]$Message
    )

    Write-Host "==> $Message"
}

function Write-WarningStep {
    param(
        [string]$Message
    )

    Write-Warning $Message
}

function Normalize-Version {
    param(
        [string]$RawVersion
    )

    if ([string]::IsNullOrWhiteSpace($RawVersion) -or $RawVersion -eq "latest") {
        return "latest"
    }

    if ($RawVersion.StartsWith("rust-v")) {
        return $RawVersion.Substring(6)
    }

    if ($RawVersion.StartsWith("v")) {
        return $RawVersion.Substring(1)
    }

    return $RawVersion
}

function Assert-ValidReleaseVersion {
    param(
        [string]$Version
    )

    # Codex SemVer plus the optional Lumi canary suffix, e.g. 0.147.0-lumi.1.
    if ($Version -cne "latest" -and $Version -cnotmatch "^[0-9]+\.[0-9]+\.[0-9]+(?:-alpha(?:\.[0-9]+){0,2}|-beta(?:\.[0-9]+)?)?(?:-lumi\.[0-9]+)?$") {
        throw "Invalid Codex release version: $Version. Expected latest or x.y.z[-alpha[.N[.M]]|-beta[.N]][-lumi.N]."
    }
}

function Find-ReleaseAssetMetadata {
    param(
        [string]$AssetName,
        [object]$ReleaseMetadata,
        [string]$ReleaseVersion
    )

    $asset = $ReleaseMetadata.assets | Where-Object { $_.name -eq $AssetName } | Select-Object -First 1
    if ($null -eq $asset) {
        return $null
    }

    $digestMatch = [regex]::Match([string]$asset.digest, "^sha256:([0-9a-fA-F]{64})$")
    if (-not $digestMatch.Success) {
        throw "Could not find SHA-256 digest for release asset $AssetName."
    }

    return [PSCustomObject]@{
        Url = "$ReleasesDownloadBase/rust-v$ReleaseVersion/$AssetName"
        Sha256 = $digestMatch.Groups[1].Value.ToLowerInvariant()
    }
}

function Invoke-WebRequestWithFallback {
    param(
        [object]$Metadata,
        [string]$OutFile,
        [string]$ExpectedDigest,
        [string]$AssetName,
        [string]$ReleaseVersion,
        [string]$RequiredManifestAsset
    )

    try {
        Invoke-WebRequest -UseBasicParsing -Uri $Metadata.Url -OutFile $OutFile -TimeoutSec $ReleasesAssetTimeoutSec
        Test-ArchiveDigest -ArchivePath $OutFile -ExpectedDigest $ExpectedDigest
        if (-not [string]::IsNullOrWhiteSpace($RequiredManifestAsset)) {
            $null = Get-PackageArchiveDigest -ManifestPath $OutFile -AssetName $RequiredManifestAsset
        }
    } catch {
        # GitHub Releases is the only source, so re-resolving the GitHub
        # release metadata and verifying the downloaded bytes against the
        # release-asset digest is the fallback trust anchor.
        Write-WarningStep "Could not download or verify $($Metadata.Url); re-verifying against GitHub release metadata."
        $githubRelease = Resolve-ReleaseFromGitHub -NormalizedVersion $ReleaseVersion
        $githubAssetMetadata = Find-ReleaseAssetMetadata -AssetName $AssetName -ReleaseMetadata $githubRelease.Metadata -ReleaseVersion $ReleaseVersion
        if ($null -eq $githubAssetMetadata) {
            throw "Could not find GitHub release metadata for asset $AssetName."
        }
        Test-ArchiveDigest -ArchivePath $OutFile -ExpectedDigest $githubAssetMetadata.Sha256
        if (-not [string]::IsNullOrWhiteSpace($RequiredManifestAsset)) {
            $null = Get-PackageArchiveDigest -ManifestPath $OutFile -AssetName $RequiredManifestAsset
        }
    }
}

function Resolve-ReleaseAssetSelection {
    param(
        [object]$ResolvedRelease,
        [string]$Target
    )

    $version = $ResolvedRelease.Version
    $releaseMetadata = $ResolvedRelease.Metadata
    $packageAsset = "codex-package-$Target.tar.gz"
    $checksumAsset = "codex-package_SHA256SUMS"

    $packageMetadata = Find-ReleaseAssetMetadata -AssetName $packageAsset -ReleaseMetadata $releaseMetadata -ReleaseVersion $version
    $checksumMetadata = Find-ReleaseAssetMetadata -AssetName $checksumAsset -ReleaseMetadata $releaseMetadata -ReleaseVersion $version
    if ($null -eq $packageMetadata -or $null -eq $checksumMetadata) {
        throw "Could not find the canonical package or checksum manifest for Lumi Codex $version (target $Target). The release is incomplete; nothing was installed and no legacy fallback is attempted."
    }

    return [PSCustomObject]@{
        PackageAsset = $packageAsset
        PackageMetadata = $packageMetadata
        ChecksumMetadata = $checksumMetadata
    }
}

function Test-ArchiveDigest {
    param(
        [string]$ArchivePath,
        [string]$ExpectedDigest
    )

    $actualDigest = (Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualDigest -ne $ExpectedDigest) {
        throw "Downloaded Lumi Codex archive checksum did not match expected digest. Expected $ExpectedDigest but got $actualDigest."
    }
}

function Get-PackageArchiveDigest {
    param(
        [string]$ManifestPath,
        [string]$AssetName
    )

    $escapedAssetName = [regex]::Escape($AssetName)
    foreach ($line in Get-Content -LiteralPath $ManifestPath) {
        $match = [regex]::Match($line, "^\s*([0-9a-fA-F]{64})\s+$escapedAssetName\s*$")
        if ($match.Success) {
            return $match.Groups[1].Value.ToLowerInvariant()
        }
    }

    throw "Could not find SHA-256 digest for $AssetName in codex-package_SHA256SUMS."
}

function Invoke-WithInstallLock {
    param(
        [string]$LockPath,
        [scriptblock]$Script
    )

    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $LockPath) | Out-Null
    $lock = $null
    while ($null -eq $lock) {
        try {
            $lock = [System.IO.File]::Open(
                $LockPath,
                [System.IO.FileMode]::OpenOrCreate,
                [System.IO.FileAccess]::ReadWrite,
                [System.IO.FileShare]::None
            )
        } catch [System.IO.IOException] {
            Start-Sleep -Milliseconds 250
        }
    }
    try {
        & $Script
    } finally {
        $lock.Dispose()
    }
}

function Remove-StaleInstallArtifacts {
    param(
        [string]$ReleasesDir
    )

    if (Test-Path -LiteralPath $ReleasesDir -PathType Container) {
        Get-ChildItem -LiteralPath $ReleasesDir -Force -Directory -Filter ".staging.*" -ErrorAction SilentlyContinue |
            Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Resolve-VersionFromReleaseMetadata {
    param(
        [object]$ReleaseMetadata
    )

    if (-not $ReleaseMetadata.tag_name) {
        throw "Failed to resolve the latest Lumi Codex release version."
    }

    $resolvedVersion = Normalize-Version -RawVersion $ReleaseMetadata.tag_name
    Assert-ValidReleaseVersion -Version $resolvedVersion
    return $resolvedVersion
}

function Resolve-ReleaseFromGitHub {
    param(
        [string]$NormalizedVersion
    )

    if ($NormalizedVersion -eq "latest") {
        $requestedRelease = "latest"
        $metadataUri = "$ReleasesApiBase/releases/latest"
    } else {
        $resolvedVersion = $NormalizedVersion
        $requestedRelease = $resolvedVersion
        $metadataUri = "$ReleasesApiBase/releases/tags/rust-v$resolvedVersion"
    }

    try {
        $releaseMetadata = Invoke-RestMethod -Uri $metadataUri
    } catch {
        if ($NormalizedVersion -eq "latest") {
            throw "Could not resolve a stable Lumi Codex release. GitHub excludes prereleases from /releases/latest; pin a canary with -Release x.y.z-lumi.N. $($_.Exception.Message)"
        }
        throw "Could not fetch GitHub release metadata for Lumi Codex $requestedRelease. GitHub API may be unavailable or rate limited. $($_.Exception.Message)"
    }

    if ($NormalizedVersion -eq "latest") {
        $resolvedVersion = Resolve-VersionFromReleaseMetadata -ReleaseMetadata $releaseMetadata
    }

    return [PSCustomObject]@{
        Version = $resolvedVersion
        Metadata = $releaseMetadata
    }
}

function Resolve-Release {
    $normalizedVersion = Normalize-Version -RawVersion $Release
    Assert-ValidReleaseVersion -Version $normalizedVersion

    return Resolve-ReleaseFromGitHub -NormalizedVersion $normalizedVersion
}

function Get-VersionFromBinary {
    param(
        [string]$CodexPath
    )

    if (-not (Test-Path -LiteralPath $CodexPath -PathType Leaf)) {
        return $null
    }

    try {
        $versionOutput = & $CodexPath --version 2>$null
    } catch {
        return $null
    }

    if ($versionOutput -match '([0-9][0-9A-Za-z.+-]*)$') {
        return $matches[1]
    }

    return $null
}

function Get-CurrentInstalledVersion {
    param(
        [string]$StandaloneCurrentDir
    )

    $standaloneVersion = Get-VersionFromBinary -CodexPath (Join-Path $StandaloneCurrentDir "bin\codex.exe")
    if (-not [string]::IsNullOrWhiteSpace($standaloneVersion)) {
        return $standaloneVersion
    }

    $standaloneVersion = Get-VersionFromBinary -CodexPath (Join-Path $StandaloneCurrentDir "codex.exe")
    if (-not [string]::IsNullOrWhiteSpace($standaloneVersion)) {
        return $standaloneVersion
    }

    return $null
}

function Add-JunctionSupportType {
    if (([System.Management.Automation.PSTypeName]'CodexInstaller.Junction').Type) {
        return
    }

    Add-Type -TypeDefinition @"
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace CodexInstaller
{
    public static class Junction
    {
        private const uint GENERIC_WRITE = 0x40000000;
        private const uint FILE_SHARE_READ = 0x00000001;
        private const uint FILE_SHARE_WRITE = 0x00000002;
        private const uint FILE_SHARE_DELETE = 0x00000004;
        private const uint OPEN_EXISTING = 3;
        private const uint FILE_FLAG_BACKUP_SEMANTICS = 0x02000000;
        private const uint FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000;
        private const uint FSCTL_SET_REPARSE_POINT = 0x000900A4;
        private const uint IO_REPARSE_TAG_MOUNT_POINT = 0xA0000003;
        private const int HeaderLength = 20;

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern SafeFileHandle CreateFileW(
            string lpFileName,
            uint dwDesiredAccess,
            uint dwShareMode,
            IntPtr lpSecurityAttributes,
            uint dwCreationDisposition,
            uint dwFlagsAndAttributes,
            IntPtr hTemplateFile);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool DeviceIoControl(
            SafeFileHandle hDevice,
            uint dwIoControlCode,
            byte[] lpInBuffer,
            int nInBufferSize,
            IntPtr lpOutBuffer,
            int nOutBufferSize,
            out int lpBytesReturned,
            IntPtr lpOverlapped);

        public static void SetTarget(string linkPath, string targetPath)
        {
            string substituteName = "\\??\\" + Path.GetFullPath(targetPath);
            byte[] substituteNameBytes = Encoding.Unicode.GetBytes(substituteName);
            if (substituteNameBytes.Length > ushort.MaxValue - HeaderLength) {
                throw new ArgumentException("Junction target path is too long.", "targetPath");
            }

            byte[] reparseBuffer = new byte[substituteNameBytes.Length + HeaderLength];
            WriteUInt32(reparseBuffer, 0, IO_REPARSE_TAG_MOUNT_POINT);
            WriteUInt16(reparseBuffer, 4, checked((ushort)(substituteNameBytes.Length + 12)));
            WriteUInt16(reparseBuffer, 8, 0);
            WriteUInt16(reparseBuffer, 10, checked((ushort)substituteNameBytes.Length));
            WriteUInt16(reparseBuffer, 12, checked((ushort)(substituteNameBytes.Length + 2)));
            WriteUInt16(reparseBuffer, 14, 0);
            Buffer.BlockCopy(substituteNameBytes, 0, reparseBuffer, 16, substituteNameBytes.Length);

            using (SafeFileHandle handle = CreateFileW(
                linkPath,
                GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                IntPtr.Zero,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                IntPtr.Zero))
            {
                if (handle.IsInvalid) {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }

                int bytesReturned;
                if (!DeviceIoControl(
                    handle,
                    FSCTL_SET_REPARSE_POINT,
                    reparseBuffer,
                    reparseBuffer.Length,
                    IntPtr.Zero,
                    0,
                    out bytesReturned,
                    IntPtr.Zero))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
            }
        }

        private static void WriteUInt16(byte[] buffer, int offset, ushort value)
        {
            buffer[offset] = (byte)value;
            buffer[offset + 1] = (byte)(value >> 8);
        }

        private static void WriteUInt32(byte[] buffer, int offset, uint value)
        {
            buffer[offset] = (byte)value;
            buffer[offset + 1] = (byte)(value >> 8);
            buffer[offset + 2] = (byte)(value >> 16);
            buffer[offset + 3] = (byte)(value >> 24);
        }
    }
}
"@
}

function Set-JunctionTarget {
    param(
        [string]$LinkPath,
        [string]$TargetPath
    )

    Add-JunctionSupportType
    [CodexInstaller.Junction]::SetTarget($LinkPath, $TargetPath)
}

function Test-IsJunction {
    param(
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        return $false
    }

    $item = Get-Item -LiteralPath $Path -Force
    return ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -and $item.LinkType -eq "Junction"
}

function Ensure-Junction {
    param(
        [string]$LinkPath,
        [string]$TargetPath,
        [string]$InstallerOwnedTargetPrefix
    )

    if (-not (Test-Path -LiteralPath $LinkPath)) {
        New-Item -ItemType Junction -Path $LinkPath -Target $TargetPath | Out-Null
        return
    }

    $item = Get-Item -LiteralPath $LinkPath -Force
    if (Test-IsJunction -Path $LinkPath) {
        $existingTarget = [string]$item.Target
        if (-not [string]::IsNullOrWhiteSpace($InstallerOwnedTargetPrefix)) {
            $ownedTargetPrefix = $InstallerOwnedTargetPrefix.TrimEnd("\")
            if (-not $existingTarget.StartsWith($ownedTargetPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
                throw "Refusing to retarget junction at $LinkPath because it is not managed by this installer."
            }
        }
        if ($existingTarget.Equals($TargetPath, [System.StringComparison]::OrdinalIgnoreCase)) {
            return
        }

        # Keep the path itself in place and only retarget the junction. That
        # avoids a gap where current disappears during an update.
        Set-JunctionTarget -LinkPath $LinkPath -TargetPath $TargetPath
        return
    }

    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "Refusing to replace non-junction reparse point at $LinkPath."
    }

    if ($item.PSIsContainer) {
        if ((Get-ChildItem -LiteralPath $LinkPath -Force | Select-Object -First 1) -ne $null) {
            throw "Refusing to replace non-empty directory at $LinkPath with a junction."
        }

        Remove-Item -LiteralPath $LinkPath -Force
        New-Item -ItemType Junction -Path $LinkPath -Target $TargetPath | Out-Null
        return
    }

    throw "Refusing to replace file at $LinkPath with a junction."
}

function Test-PackageContentsAreComplete {
    param(
        [string]$PackageDir
    )

    if (-not (Test-Path -LiteralPath $PackageDir -PathType Container)) {
        return $false
    }

    $expectedFiles = @(
        "codex-package.json",
        "bin\codex.exe",
        "bin\codex-code-mode-host.exe",
        "codex-path\rg.exe",
        "codex-resources\codex-command-runner.exe",
        "codex-resources\codex-windows-sandbox-setup.exe"
    )
    foreach ($name in $expectedFiles) {
        if (-not (Test-Path -LiteralPath (Join-Path $PackageDir $name) -PathType Leaf)) {
            return $false
        }
    }

    return $true
}

function Test-ReleaseIsComplete {
    param(
        [string]$ReleaseDir,
        [string]$ExpectedVersion,
        [string]$ExpectedTarget
    )

    if (-not (Test-PackageContentsAreComplete -PackageDir $ReleaseDir)) {
        return $false
    }

    return (Split-Path -Leaf $ReleaseDir) -eq "$ExpectedVersion-$ExpectedTarget" -and
        (Get-VersionFromBinary -CodexPath (Join-Path $ReleaseDir "bin\codex.exe")) -ceq $ExpectedVersion
}

function Assert-AbsolutePath {
    param(
        [string]$Path,
        [string]$Label
    )

    if (-not [System.IO.Path]::IsPathRooted($Path)) {
        throw "$Label must be an absolute path (got: $Path)"
    }

    $controlChars = [char[]](@(0..31) + @(127) | ForEach-Object { [char]$_ })
    if (
        $Path.IndexOfAny($controlChars) -ge 0 -or
        $Path.Contains("'") -or
        $Path.Contains('"') -or
        $Path.Contains("%") -or
        $Path.Contains("!")
    ) {
        throw "$Label contains characters that cannot be represented safely in the lumi-codex.cmd launcher; refusing."
    }
}

function Install-VisibleLauncher {
    param(
        [string]$LauncherPath,
        [string]$LumiRoot,
        [string]$VisibleBinDir
    )

    New-Item -ItemType Directory -Force -Path $VisibleBinDir | Out-Null
    # Tiny cmd launcher that runs the real package entrypoint so packaged
    # resources stay adjacent to the actual binary.
    $desired = "@echo off`r`n`"$LumiRoot\current\bin\codex.exe`" %*`r`n"

    if (Test-Path -LiteralPath $LauncherPath) {
        $item = Get-Item -LiteralPath $LauncherPath -Force
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "Refusing to replace reparse point at $LauncherPath (not a Lumi Codex launcher)."
        }
        if ($item.PSIsContainer) {
            throw "Refusing to replace directory at $LauncherPath."
        }
        $existing = [System.IO.File]::ReadAllText($LauncherPath)
        if ($existing -cne $desired) {
            throw "Refusing to overwrite unexpected file at $LauncherPath; remove it or point LUMI_INSTALL_DIR elsewhere."
        }
    }

    $tmpLauncher = Join-Path $VisibleBinDir (".lumi-codex." + $PID + ".tmp")
    [System.IO.File]::WriteAllText($tmpLauncher, $desired)
    Move-Item -LiteralPath $tmpLauncher -Destination $LauncherPath -Force
}

function Test-VisibleLauncher {
    param(
        [string]$LauncherPath
    )

    & $LauncherPath --version *> $null
    if ($LASTEXITCODE -ne 0) {
        throw "Installed Lumi Codex launcher failed verification: $LauncherPath --version"
    }
}

if ($env:OS -ne "Windows_NT") {
    Write-Error "install.ps1 supports Windows only. Use install.sh on macOS or Linux."
    exit 1
}

if (-not [Environment]::Is64BitOperatingSystem) {
    Write-Error "Lumi Codex requires a 64-bit version of Windows."
    exit 1
}

$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
$platformLabel = $null
if ([string]::IsNullOrWhiteSpace($Target)) {
    switch ($architecture) {
        "Arm64" {
            $Target = "aarch64-pc-windows-msvc"
            $platformLabel = "Windows (ARM64)"
        }
        "X64" {
            $Target = "x86_64-pc-windows-msvc"
            $platformLabel = "Windows (x64)"
        }
        default {
            Write-Error "Unsupported architecture: $architecture"
            exit 1
        }
    }
} else {
    switch ($Target) {
        "aarch64-pc-windows-msvc" {
            $platformLabel = "Windows (ARM64)"
        }
        "x86_64-pc-windows-msvc" {
            $platformLabel = "Windows (x64)"
        }
        default {
            Write-Error "Unsupported target: $Target. Supported targets: $($TargetAllowlist -join ', ')."
            exit 1
        }
    }
}

$lumiRoot = if ([string]::IsNullOrWhiteSpace($env:LUMI_ROOT)) {
    Join-Path $env:LOCALAPPDATA "lumi-codex"
} else {
    $env:LUMI_ROOT
}
Assert-AbsolutePath -Path $lumiRoot -Label "LUMI_ROOT"
if (Test-Path -LiteralPath $lumiRoot) {
    $rootItem = Get-Item -LiteralPath $lumiRoot -Force
    if ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "Refusing to operate on symlinked or junction root $lumiRoot (remove it or point LUMI_ROOT at a real directory)."
    }
}

$visibleBinDir = if ([string]::IsNullOrWhiteSpace($env:LUMI_INSTALL_DIR)) {
    Join-Path $env:LOCALAPPDATA "Programs\Lumi\Codex\bin"
} else {
    $env:LUMI_INSTALL_DIR
}
Assert-AbsolutePath -Path $visibleBinDir -Label "LUMI_INSTALL_DIR"
if (Test-Path -LiteralPath $visibleBinDir) {
    $binItem = Get-Item -LiteralPath $visibleBinDir -Force
    if ($binItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "Refusing to install the launcher through symlinked or junction directory $visibleBinDir."
    }
}
$launcherPath = Join-Path $visibleBinDir "lumi-codex.cmd"

$releasesDir = Join-Path $lumiRoot "releases"
$currentDir = Join-Path $lumiRoot "current"
$lockPath = Join-Path $lumiRoot "install.lock"

$currentVersion = Get-CurrentInstalledVersion -StandaloneCurrentDir $currentDir
$resolvedRelease = Resolve-Release
$resolvedVersion = $resolvedRelease.Version
$releaseName = "$resolvedVersion-$Target"
$releaseDir = Join-Path $releasesDir $releaseName

if (-not [string]::IsNullOrWhiteSpace($currentVersion) -and $currentVersion -ne $resolvedVersion) {
    Write-Step "Updating Lumi Codex CLI from $currentVersion to $resolvedVersion"
} elseif (-not [string]::IsNullOrWhiteSpace($currentVersion)) {
    Write-Step "Updating Lumi Codex CLI"
} else {
    Write-Step "Installing Lumi Codex CLI"
}
Write-Step "Detected platform: $platformLabel"
Write-Step "Resolved version: $resolvedVersion"

$checksumAsset = "codex-package_SHA256SUMS"
$assetSelection = Resolve-ReleaseAssetSelection -ResolvedRelease $resolvedRelease -Target $Target
$packageAsset = $assetSelection.PackageAsset
$packageMetadata = $assetSelection.PackageMetadata
$checksumMetadata = $assetSelection.ChecksumMetadata
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("lumi-codex-install-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null

try {
    Invoke-WithInstallLock -LockPath $lockPath -Script {
        Remove-StaleInstallArtifacts -ReleasesDir $releasesDir

        if (-not (Test-ReleaseIsComplete -ReleaseDir $releaseDir -ExpectedVersion $resolvedVersion -ExpectedTarget $Target)) {
            if (Test-Path -LiteralPath $releaseDir) {
                Write-WarningStep "Found incomplete existing release at $releaseDir. Reinstalling."
            }

            $archivePath = Join-Path $tempDir $packageAsset
            $checksumPath = Join-Path $tempDir $checksumAsset
            $stagingDir = Join-Path $releasesDir ".staging.$releaseName.$PID"

            Write-Step "Downloading Lumi Codex CLI"
            Invoke-WebRequestWithFallback -Metadata $checksumMetadata -OutFile $checksumPath -ExpectedDigest $checksumMetadata.Sha256 -AssetName $checksumAsset -ReleaseVersion $resolvedVersion -RequiredManifestAsset $packageAsset
            $expectedPackageDigest = Get-PackageArchiveDigest -ManifestPath $checksumPath -AssetName $packageAsset
            Invoke-WebRequestWithFallback -Metadata $packageMetadata -OutFile $archivePath -ExpectedDigest $expectedPackageDigest -AssetName $packageAsset -ReleaseVersion $resolvedVersion

            New-Item -ItemType Directory -Force -Path $releasesDir | Out-Null
            if (Test-Path -LiteralPath $stagingDir) {
                Remove-Item -LiteralPath $stagingDir -Recurse -Force
            }
            New-Item -ItemType Directory -Force -Path $stagingDir | Out-Null
            tar -xzf $archivePath -C $stagingDir
            if (-not (Test-PackageContentsAreComplete -PackageDir $stagingDir)) {
                throw "Downloaded Lumi Codex package archive did not contain the expected package layout."
            }

            # Fail closed instead of deleting a foreign file or reparse point
            # at the release path; a plain directory is our own incomplete
            # state and is replaced.
            if (Test-Path -LiteralPath $releaseDir) {
                $releaseItem = Get-Item -LiteralPath $releaseDir -Force
                if ($releaseItem.Attributes -band [IO.FileAttributes]::ReparsePoint -or -not $releaseItem.PSIsContainer) {
                    throw "Refusing to replace unexpected non-directory at $releaseDir"
                }
                Remove-Item -LiteralPath $releaseDir -Recurse -Force
            }
            Move-Item -LiteralPath $stagingDir -Destination $releaseDir
        }

        if (-not (Test-ReleaseIsComplete -ReleaseDir $releaseDir -ExpectedVersion $resolvedVersion -ExpectedTarget $Target)) {
            throw "Installed Codex command did not report expected version $resolvedVersion."
        }

        New-Item -ItemType Directory -Force -Path $lumiRoot | Out-Null
        Ensure-Junction -LinkPath $currentDir -TargetPath $releaseDir -InstallerOwnedTargetPrefix $releasesDir
        Install-VisibleLauncher -LauncherPath $launcherPath -LumiRoot $lumiRoot -VisibleBinDir $visibleBinDir
        Test-VisibleLauncher -LauncherPath $launcherPath
    }
} finally {
    Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
}

$onPath = $null -ne ($env:Path.Split(";", [System.StringSplitOptions]::RemoveEmptyEntries) |
    Where-Object { $_.TrimEnd("\") -ieq $visibleBinDir.TrimEnd("\") })
if ($onPath) {
    Write-Step "Current terminal: lumi-codex"
    Write-Step "Future terminals: open a new PowerShell window and run: lumi-codex"
} else {
    Write-Step "Add $visibleBinDir to your PATH, or run the launcher directly:"
    Write-Step "  $launcherPath"
}

Write-Host "Lumi Codex CLI $resolvedVersion installed successfully."
