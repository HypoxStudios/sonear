<#
.SYNOPSIS
Signs the setup file of a draft release so Sonear's updater will accept it.

.DESCRIPTION
Downloads sonear-<version>-setup.exe from the draft release, signs it with
minisign (it asks for the key's password) and uploads the .minisig next to it.
It does not publish the release.

.PARAMETER Tag
The release tag, such as v0.2.0.

.PARAMETER KeyFile
A minisign secret key file. Without it the key is read from the Bitwarden note
"Sonear update signing key" (the Bitwarden CLI must be unlocked).

.EXAMPLE
.\scripts\sign-release.ps1 -Tag v0.2.0
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^v\d+\.\d+\.\d+$')]
    [string]$Tag,

    [string]$KeyFile
)

$ErrorActionPreference = 'Stop'

$repo = 'HypoxStudios/sonear'
$noteName = 'Sonear update signing key'
$publicKey = Join-Path $PSScriptRoot 'sonear-update.pub'

function Require-Tool([string]$name, [string]$hint) {
    if (-not (Get-Command $name -ErrorAction SilentlyContinue)) {
        throw "$name was not found. $hint"
    }
}

function Assert-Succeeded([string]$what) {
    if ($LASTEXITCODE -ne 0) {
        throw "$what failed (exit code $LASTEXITCODE)."
    }
}

Require-Tool 'minisign' 'Install it with: winget install jedisct1.minisign'
Require-Tool 'gh' 'Install it with: winget install GitHub.cli'

if (-not $KeyFile) {
    Require-Tool 'bw' 'Install the Bitwarden CLI, or pass -KeyFile.'
}

$version = $Tag.Substring(1)
$setup = "sonear-$version-setup.exe"
$work = Join-Path ([IO.Path]::GetTempPath()) ('sonear-sign-' + [guid]::NewGuid().ToString('N'))
$tempKey = $null

New-Item -ItemType Directory -Path $work | Out-Null

try {
    Write-Host "Downloading $setup from $Tag"
    & gh release download $Tag --repo $repo --pattern $setup --dir $work
    Assert-Succeeded 'Downloading the setup file'

    $exe = Join-Path $work $setup

    if (-not (Test-Path -LiteralPath $exe)) {
        throw "$setup is not attached to $Tag."
    }

    if ($KeyFile) {
        if (-not (Test-Path -LiteralPath $KeyFile)) {
            throw "The key file $KeyFile does not exist."
        }

        $key = (Resolve-Path -LiteralPath $KeyFile).Path
    }
    else {
        $notes = (& bw get notes $noteName) -join "`n"
        Assert-Succeeded 'Reading the key from Bitwarden (is it unlocked? Set BW_SESSION)'

        if ([string]::IsNullOrWhiteSpace($notes)) {
            throw "The Bitwarden note '$noteName' is empty."
        }

        $tempKey = Join-Path ([IO.Path]::GetTempPath()) ('sonear-key-' + [guid]::NewGuid().ToString('N') + '.key')
        [IO.File]::WriteAllText($tempKey, $notes.Trim() + "`n", (New-Object System.Text.UTF8Encoding($false)))
        $notes = $null
        $key = $tempKey
    }

    Write-Host 'Signing. minisign asks for the key password.'
    & minisign -S -s $key -m $exe -t "sonear $version"
    Assert-Succeeded 'Signing'

    $signature = "$exe.minisig"

    if (-not (Test-Path -LiteralPath $signature)) {
        throw 'minisign did not write a signature file.'
    }

    if (Test-Path -LiteralPath $publicKey) {
        & minisign -V -p $publicKey -m $exe
        Assert-Succeeded 'Checking the signature against scripts/sonear-update.pub'
    }
    else {
        Write-Warning 'scripts/sonear-update.pub does not exist, so the signature was not checked.'
    }

    Write-Host 'Uploading the signature'
    & gh release upload $Tag $signature --repo $repo --clobber
    Assert-Succeeded 'Uploading the signature'

    Write-Host "Done. $Tag now has $setup.minisig. The draft is not published yet."
}
finally {
    if ($tempKey -and (Test-Path -LiteralPath $tempKey)) {
        Remove-Item -LiteralPath $tempKey -Force -ErrorAction SilentlyContinue
    }

    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
