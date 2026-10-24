<#
Build SolidCraft on the Linux build server instead of this laptop.

The current branch's HEAD is archived with `git archive`, copied to the server,
extracted over ~/solidcraft-build (target/ is kept warm), and built there with
`cargo xwin` (cross-compile to Windows MSVC). The exe and its PDB are copied
back to target\server-build\ and the app is launched with a control channel.

Modes:
  build (default)  sync + xwin build + copy back + launch (--sample --control PORT)
  test             sync + native cargo test of the library crates on the server
  check            sync + clippy -D warnings + wasm32 check of the library crates
  all              check + test + build + copy back + launch

Examples:
  scripts\server-build.ps1                    # build and run
  scripts\server-build.ps1 -Mode test         # fast test loop on the server
  scripts\server-build.ps1 -Mode all -NoRun   # full gate, no launch
#>
param(
    [ValidateSet("build", "test", "check", "all")]
    [string]$Mode = "build",
    [string]$Server = "user@192.168.0.99",
    [string]$RemoteDir = "solidcraft-build",
    [string]$XwinCache = "solidcraft-remote/xwin/xwin",
    [int]$ControlPort = 0,
    [switch]$NoRun
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$outDir = Join-Path $repo "target\server-build"
$started = Get-Date

# Library crates only: the apps need Windows-only or system GUI bits the server
# may not have; the xwin build covers the app for Windows.
$libCrates = @(
    "-p", "solidcraft-geom", "-p", "solidcraft-sketch", "-p", "solidcraft-kernel",
    "-p", "solidcraft-render", "-p", "solidcraft-doc", "-p", "solidcraft-io",
    "-p", "solidcraft-tooling", "-p", "solidcraft-engine"
)

function Invoke-Step([string]$name, [string]$command) {
    Write-Host "==> ${name}" -ForegroundColor Cyan
    $t = Get-Date
    Invoke-Expression $command
    if ($LASTEXITCODE -ne 0) { throw "${name} failed (exit $LASTEXITCODE)" }
    Write-Host ("--- {0}: {1:mm\:ss}" -f $name, ((Get-Date) - $t)) -ForegroundColor DarkGray
}

# 1. Archive HEAD and ship it. Untracked files are not included: commit first.
$tar = Join-Path $env:TEMP "solidcraft-tree.tar"
Invoke-Step "archive HEAD" "git -C '$repo' archive --format=tar --output '$tar' HEAD"
Invoke-Step "scp tree to ${Server}" "scp -q '$tar' '${Server}:~/tree.tar'"
Remove-Item $tar -ErrorAction SilentlyContinue

# 2. Extract on the server (everything except target/ and the stamp repo is
#    replaced, so the tree matches HEAD exactly) and run the requested steps
#    there. The tiny git repo gives crates/io's build script a stable commit to
#    stamp; without one ("unknown") the model-cache tests refuse to trust saves.
$remote = "cd ~ && mkdir -p $RemoteDir && cd $RemoteDir && " +
    "find . -maxdepth 1 -mindepth 1 ! -name target ! -name .git -exec rm -rf {} + && tar -xf ~/tree.tar && " +
    "[ -d .git ] || { git init -q . && git -c user.email=build@solidcraft.local -c user.name=server-build commit -q --allow-empty -m 'server-build sync'; }"
$ssh = "ssh -o BatchMode=yes $Server"

if ($Mode -in @("check", "all")) {
    Invoke-Step "clippy (server)" "$ssh ""$remote && cargo clippy $($libCrates -join ' ') --all-targets --profile ci -- -D warnings"""
    Invoke-Step "wasm check (server)" "$ssh ""$remote && cargo check $($libCrates -join ' ') --target wasm32-unknown-unknown --profile ci"""
}
if ($Mode -in @("test", "all")) {
    Invoke-Step "tests (server)" "$ssh ""$remote && cargo test $($libCrates -join ' ') --profile ci"""
}
if ($Mode -in @("build", "all")) {
    Invoke-Step "xwin build (server)" "$ssh ""$remote && XWIN_CACHE_DIR=~/$XwinCache cargo xwin build --target x86_64-pc-windows-msvc -p solidcraft"""

    # 3. Copy the exe and its PDB back (the PDB lets VS Code debug the cross build).
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    $rt = "~/solidcraft-build/target/x86_64-pc-windows-msvc/debug"
    Invoke-Step "copy solidcraft.exe back" "scp -q '${Server}:$rt/solidcraft.exe' '$outDir\solidcraft.exe'"
    # The PDB is optional: a build can be configured without line tables.
    scp -q "${Server}:$rt/solidcraft.pdb" "$outDir\solidcraft.pdb" 2>$null
    if ($LASTEXITCODE -ne 0) { Write-Host "    (no PDB copied; debugging will be limited)" -ForegroundColor Yellow }

    # 4. Launch with a control channel on a free port.
    if (-not $NoRun) {
        if ($ControlPort -eq 0) {
            $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
            $listener.Start(); $ControlPort = $listener.LocalEndpoint.Port; $listener.Stop()
        }
        Start-Process -FilePath (Join-Path $outDir "solidcraft.exe") `
            -ArgumentList @("--sample", "--control", "$ControlPort") -WorkingDirectory $repo
        $token = Join-Path $env:LOCALAPPDATA "SolidCraft\control\$ControlPort.token"
        Write-Host "==> launched: control channel on 127.0.0.1:$ControlPort" -ForegroundColor Green
        Write-Host "    token: $token"
        Set-Content -Path (Join-Path $outDir "last-control-port.txt") -Value "$ControlPort"
    }
}

Write-Host ("==> done in {0:mm\:ss}" -f ((Get-Date) - $started)) -ForegroundColor Green
