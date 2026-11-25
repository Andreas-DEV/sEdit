# Bygger sEdit, kopierer den til en fast placering og tilfoejer den til "Aabn med" i Stifinder.
# Alt skrives under den aktuelle bruger (HKCU), saa der kraeves ikke administrator.
# Standardprogrammer aendres ikke; sEdit bliver kun et valg i "Aabn med".
#
#   .\scripts\install.ps1              installer eller opdater
#   .\scripts\install.ps1 -Uninstall   fjern igen
param([switch]$Uninstall)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\sEdit'
$exe = Join-Path $installDir 'sedit.exe'
$classes = 'HKCU:\Software\Classes'
$progId = 'sEdit.TextFile'
$appKey = "$classes\Applications\sedit.exe"

# Filtyper hvor sEdit vises direkte i "Aabn med". Windows regner alt efter sidste punktum
# som filtypen, saa ".env.local" er typen ".local". For alle andre typer findes sEdit
# under "Vaelg en anden app".
$extensions = '.env', '.local', '.development', '.production', '.staging', '.test', '.example',
              '.txt', '.log', '.ini', '.cfg', '.conf'

function Update-Shell {
    Add-Type -Namespace SEdit -Name Shell -MemberDefinition '[DllImport("shell32.dll")] public static extern void SHChangeNotify(int eventId, uint flags, IntPtr item1, IntPtr item2);'
    [SEdit.Shell]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)
}

if ($Uninstall) {
    foreach ($ext in $extensions) {
        $key = "$classes\$ext\OpenWithProgids"
        if (Test-Path -LiteralPath $key) { Remove-ItemProperty -LiteralPath $key -Name $progId -ErrorAction SilentlyContinue }
    }
    foreach ($key in "$classes\$progId", $appKey) {
        if (Test-Path -LiteralPath $key) { Remove-Item -LiteralPath $key -Recurse -Force }
    }
    if (Test-Path -LiteralPath $installDir) { Remove-Item -LiteralPath $installDir -Recurse -Force }
    Update-Shell
    Write-Host 'sEdit er fjernet fra "Aabn med".'
    return
}

$cargo = Get-Command cargo -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Source
if (-not $cargo) { $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
& $cargo build --release --manifest-path (Join-Path $repo 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'cargo build fejlede.' }

New-Item -ItemType Directory -Force -Path $installDir | Out-Null
$built = Join-Path $repo 'target\release\sedit.exe'
$old = Join-Path $installDir 'sedit.old.exe'
Remove-Item -LiteralPath $old -Force -ErrorAction SilentlyContinue
$running = $false
try {
    Copy-Item -LiteralPath $built -Destination $exe -Force
} catch {
    # sEdit koerer og laaser filen. En koerende exe maa gerne omdoebes, saa den gamle
    # flyttes til side og fortsaetter uforstyrret; den nye bruges fra naeste start.
    Rename-Item -LiteralPath $exe -NewName 'sedit.old.exe'
    Copy-Item -LiteralPath $built -Destination $exe -Force
    $running = $true
}

$command = "`"$exe`" `"%1`""
function Set-OpenCommand($key) {
    New-Item -Path "$key\shell\open\command" -Force | Out-Null
    Set-ItemProperty -LiteralPath "$key\shell\open\command" -Name '(default)' -Value $command
}

Set-OpenCommand $appKey
Set-ItemProperty -LiteralPath $appKey -Name 'FriendlyAppName' -Value 'sEdit'
Set-OpenCommand "$classes\$progId"
Set-ItemProperty -LiteralPath "$classes\$progId" -Name '(default)' -Value 'Tekstfil'

foreach ($ext in $extensions) {
    $key = "$classes\$ext\OpenWithProgids"
    if (-not (Test-Path -LiteralPath $key)) { New-Item -Path $key -Force | Out-Null }
    New-ItemProperty -LiteralPath $key -Name $progId -Value '' -PropertyType String -Force | Out-Null
}

Update-Shell
if ($running) { Write-Host 'sEdit koerer stadig i den gamle udgave. Luk og aabn den igen for at faa den nye.' }
Write-Host "sEdit er installeret i $exe og tilfoejet til `"Aabn med`"."
