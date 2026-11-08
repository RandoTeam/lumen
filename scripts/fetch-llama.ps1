#requires -Version 5.1
param(
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root "external\llama.cpp"

if (-not (Test-Path $dest)) {
    New-Item -ItemType Directory -Force (Join-Path $root "external") | Out-Null
    git clone --depth 1 https://github.com/ggml-org/llama.cpp $dest
} elseif ($Force) {
    git -C $dest fetch --depth 1 origin
    git -C $dest reset --hard origin/master
}

# The APK links this exact commit: the last PR revision that consumes the
# block-diffusion tensors of the APEX-I Clover GGUFs.
$cloverCommit = "4838ffb0c8ade92f34d9157deea38d92758ab824"
$cloverTree = Join-Path $root "external\llama-clover-src"

if (-not (Test-Path $cloverTree)) {
    git -C $dest fetch --depth 1 origin $cloverCommit
    git -C $dest worktree add $cloverTree $cloverCommit
}

git -C $dest log -1 --format="llama.cpp mainline: %H %ad %s" --date=iso
git -C $cloverTree log -1 --format="llama.cpp clover:   %H %ad %s" --date=iso
