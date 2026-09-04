# Native Windows wrapper; building does not install, launch, tag or publish.
param(
    [string]$Target = 'x86_64-pc-windows-msvc',
    [string]$Output,
    [string]$HelperInventory,
    [switch]$Plan,
    [switch]$Check
)
$ErrorActionPreference = 'Stop'
$ProjectRoot = Split-Path -Parent $PSScriptRoot
$BuildArgs = @((Join-Path $PSScriptRoot 'build_candidate.py'), $Target)
if ($Output) { $BuildArgs += @('--output', $Output) }
if ($HelperInventory) { $BuildArgs += @('--helper-inventory', $HelperInventory) }
if ($Plan) { $BuildArgs += '--plan' }
if ($Check) { $BuildArgs += '--check' }
& python @BuildArgs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
