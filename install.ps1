# Installs the latest kiln release, or the one you name, for the current
# user, from PowerShell on Windows:
#
#   irm https://raw.githubusercontent.com/IZ737/kiln/main/install.ps1 | iex
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/IZ737/kiln/main/install.ps1))) -Version v0.2.0
#
# It downloads the release zip, checks it against the release's .sha256
# file, puts kiln.exe in ~\.kiln\bin and adds that folder to your user PATH.
# The environment variables KILN_VERSION, KILN_HOME and
# KILN_NO_MODIFY_PATH=1 do the same as the parameters. It works in Windows
# PowerShell 5.1 and PowerShell 7.

param(
    # The release to install, such as v0.2.0. The default is the latest.
    [string]$Version = $env:KILN_VERSION,
    # kiln.exe goes in <InstallDir>\bin. The default is ~\.kiln.
    [string]$InstallDir = $env:KILN_HOME,
    # Leave the user PATH alone.
    [switch]$NoModifyPath = ($env:KILN_NO_MODIFY_PATH -eq '1')
)

# Everything is inside Install-Kiln, called on the last line, so a download
# cut off halfway runs nothing. Failures throw instead of calling exit, which
# would close the window of someone running this through iex.

function Install-Kiln {
    param([string]$Version, [string]$InstallDir, [bool]$NoModifyPath)

    $ErrorActionPreference = 'Stop'
    # Invoke-WebRequest is many times slower with its progress bar in 5.1.
    $ProgressPreference = 'SilentlyContinue'
    $repo = 'IZ737/kiln'

    if ($PSVersionTable.PSEdition -eq 'Core' -and -not $IsWindows) {
        throw "this script is for Windows; on Linux and macOS run: curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/$repo/main/install.sh | sh"
    }
    # Windows PowerShell 5.1 may not offer TLS 1.2, which github.com needs.
    if ($PSVersionTable.PSVersion.Major -lt 6) {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    }

    $machine = $env:PROCESSOR_ARCHITEW6432
    if (-not $machine) { $machine = $env:PROCESSOR_ARCHITECTURE }
    switch ($machine) {
        'AMD64' { $arch = 'x86_64' }
        # Windows 11 on ARM runs x86-64 programs.
        'ARM64' { $arch = 'x86_64' }
        default { throw "there is no prebuilt kiln for $machine processors; build it from source: https://github.com/$repo#try-it" }
    }

    if (-not $Version) {
        $Version = Get-LatestVersion $repo
    }
    if (-not $Version.StartsWith('v')) { $Version = "v$Version" }

    if (-not $InstallDir) { $InstallDir = Join-Path $HOME '.kiln' }
    $bin = Join-Path $InstallDir 'bin'

    $name = "kiln-$Version-$arch-windows"
    $url = "https://github.com/$repo/releases/download/$Version/$name.zip"
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("kiln-install-" + [Guid]::NewGuid())
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        Write-Host "kiln: downloading kiln $Version for windows $arch"
        $zip = Join-Path $tmp "$name.zip"
        try {
            Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $zip
        } catch {
            # Tell a missing release apart from a release without Windows.
            try {
                Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/$repo/releases/tag/$Version" -OutFile (Join-Path $tmp 'release.html')
            } catch {
                throw "could not download release $Version; the releases are at https://github.com/$repo/releases"
            }
            throw "release $Version has no prebuilt kiln for windows $arch; build it from source: https://github.com/$repo#try-it"
        }
        $sumFile = Join-Path $tmp "$name.zip.sha256"
        try {
            Invoke-WebRequest -UseBasicParsing -Uri "$url.sha256" -OutFile $sumFile
        } catch {
            throw "could not download $url.sha256"
        }

        $expected = ((Get-Content -Raw $sumFile).Trim() -split '\s+')[0].ToLowerInvariant()
        $actual = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash.ToLowerInvariant()
        if ($expected -ne $actual) {
            throw "the download does not match its checksum (expected $expected, got $actual); try again"
        }

        Expand-Archive -Path $zip -DestinationPath $tmp -Force
        $exe = Join-Path (Join-Path $tmp $name) 'kiln.exe'
        if (-not (Test-Path $exe)) { throw "$name.zip has no kiln.exe in it" }

        New-Item -ItemType Directory -Force -Path $bin | Out-Null
        $target = Join-Path $bin 'kiln.exe'
        # A running kiln.exe cannot be overwritten but can be renamed.
        $old = "$target.old"
        if (Test-Path $old) { Remove-Item -Force $old -ErrorAction SilentlyContinue }
        if (Test-Path $target) { Move-Item -Force $target $old }
        Move-Item $exe $target
        if (Test-Path $old) { Remove-Item -Force $old -ErrorAction SilentlyContinue }
        Write-Host "kiln: installed kiln $Version to $target"
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }

    if (-not $NoModifyPath) {
        Add-ToUserPath $bin
    }
    # The current window gets it too, so kiln works straight away when this
    # ran through iex.
    if (-not (($env:Path -split ';') -contains $bin)) {
        $env:Path = "$bin;$env:Path"
    }
    Write-Host "kiln: run 'kiln --help' to get started"
}

# The latest release's tag, from where github.com/<repo>/releases/latest
# redirects. This needs no API token and has no rate limit.
function Get-LatestVersion([string]$repo) {
    try {
        $response = Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/$repo/releases/latest"
    } catch {
        throw "could not find the latest release at https://github.com/$repo/releases"
    }
    $base = $response.BaseResponse
    # PowerShell 7 keeps the final address on the request; 5.1 on the response.
    if ($base.RequestMessage) { $final = $base.RequestMessage.RequestUri } else { $final = $base.ResponseUri }
    $tag = $final.AbsolutePath.Split('/')[-1]
    if ($tag -notmatch '^v[0-9]') {
        throw "could not find the latest release at https://github.com/$repo/releases"
    }
    return $tag
}

function Add-ToUserPath([string]$dir) {
    # Read the raw value so entries such as %USERPROFILE%\bin stay as they are.
    $key = Get-Item -Path 'HKCU:\Environment'
    $path = $key.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
    $entries = @($path -split ';' | Where-Object { $_ })
    if ($entries -contains $dir) { return }
    $new = (@($dir) + $entries) -join ';'
    Set-ItemProperty -Path 'HKCU:\Environment' -Name 'Path' -Value $new -Type ExpandString
    # Setting a user variable through .NET tells open programs, such as
    # Explorer, that the environment changed, so new terminals see the PATH.
    [Environment]::SetEnvironmentVariable('KILN_INSTALLER', '1', 'User')
    [Environment]::SetEnvironmentVariable('KILN_INSTALLER', $null, 'User')
    Write-Host "kiln: added $dir to your user PATH; new terminals will find kiln"
}

try {
    Install-Kiln -Version $Version -InstallDir $InstallDir -NoModifyPath ([bool]$NoModifyPath)
} catch {
    Write-Host "kiln: error: $($_.Exception.Message)" -ForegroundColor Red
    # Run as a file (.\install.ps1, or in CI), fail with a status. Through
    # iex there is no file, and exit would close the window.
    if ($PSCommandPath) { exit 1 }
}
