# v1.0.1: ASCII-only working path via junction, so dlltool/windres
# don't choke on Chinese characters in the original project path.
if (-not $env:NINE_SNAKE_ROOT) {
    $env:NINE_SNAKE_ROOT = 'C:\dev2\nine-snake'
}
$projRoot = $env:NINE_SNAKE_ROOT
$mingw = 'C:\Users\jiuzh\AppData\Local\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin'
$protoc = 'C:\Users\jiuzh\AppData\Local\Microsoft\WinGet\Packages\Google.Protobuf_Microsoft.Winget.Source_8wekyb3d8bbwe\bin'
$env:Path = "$mingw;$protoc;$env:Path"
$env:PROTOC = 'C:\Users\jiuzh\AppData\Local\Microsoft\WinGet\Packages\Google.Protobuf_Microsoft.Winget.Source_8wekyb3d8bbwe\bin\protoc.exe'
# v1.0.1: help windres cope with non-ASCII (Chinese) characters in
# the project path.
$env:LANG = 'en_US.UTF-8'
$env:LC_ALL = 'en_US.UTF-8'
$env:TAURI_SKIP_DEVSERVER_CHECK = '1'
# v1.0.1: switch console + child process output code page to UTF-8
# so rustc / linker do not reinterpret the Chinese path as GBK.
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}
try { chcp 65001 | Out-Null } catch {}
$targetDir = Join-Path $projRoot 'target'
if (-not (Test-Path $targetDir)) { New-Item -ItemType Directory -Path $targetDir | Out-Null }
Set-Location "$projRoot\src-tauri"
try {
    Write-Host "=== testing proto file visibility ==="
    & 'C:\Users\jiuzh\AppData\Local\Microsoft\WinGet\Packages\Google.Protobuf_Microsoft.Winget.Source_8wekyb3d8bbwe\bin\protoc.exe' --version
    if (Test-Path 'proto\nine_snake.proto') {
        Write-Host "proto file exists at relative path"
    } else {
        Write-Host "proto file NOT found at relative path"
    }
    if (Test-Path 'src\grpc\proto') {
        Write-Host "out_dir exists"
    } else {
        New-Item -ItemType Directory -Path 'src\grpc\proto' -Force | Out-Null
        Write-Host "created out_dir"
    }
    $logPath = Join-Path $projRoot 'cargo_check12.log'
    & 'C:\Users\jiuzh\.cargo\bin\cargo.exe' check --lib --target x86_64-pc-windows-gnu --target-dir $targetDir 2>&1 | Tee-Object -FilePath $logPath | Select-String -Pattern '^error|Finished' | Select-Object -Last 60
} finally {
    Set-Location $projRoot
}
