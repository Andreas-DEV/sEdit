# Tegner sEdits ikon (blaa afrundet flise med et moerkt "s") og skriver assets\sedit.ico.
# Hver stoerrelse tegnes for sig, saa de smaa udgaver staar skarpt.
#
#   .\scripts\make-icon.ps1                      skriv assets\sedit.ico
#   .\scripts\make-icon.ps1 -Preview ark.png     skriv ogsaa et oversigtsbillede af alle stoerrelser
param([string]$Preview)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$repo = Split-Path -Parent $PSScriptRoot
$sizes = 16, 20, 24, 32, 40, 48, 64, 256

function New-IconBitmap([int]$size) {
    $bmp = New-Object Drawing.Bitmap $size, $size, ([Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.PixelOffsetMode = 'HighQuality'
    $g.Clear([Drawing.Color]::Transparent)

    # Appens accentfarve som flade og det moerke tema som bogstav. Den lyse flade ses
    # tydeligt paa baade en lys og en moerk proceslinje uden kant eller skygge.
    $accent = [Drawing.Color]::FromArgb(255, 76, 194, 255)
    $dark = [Drawing.Color]::FromArgb(255, 32, 32, 32)

    # Flise med afrundede hjoerner, som Windows 11's egne ikoner.
    $inset = $size / 16.0
    $side = $size - 2 * $inset
    $d = 2 * $side * 0.22
    $tile = New-Object Drawing.Drawing2D.GraphicsPath
    $tile.AddArc($inset, $inset, $d, $d, 180, 90)
    $tile.AddArc($inset + $side - $d, $inset, $d, $d, 270, 90)
    $tile.AddArc($inset + $side - $d, $inset + $side - $d, $d, $d, 0, 90)
    $tile.AddArc($inset, $inset + $side - $d, $d, $d, 90, 90)
    $tile.CloseFigure()
    $g.FillPath((New-Object Drawing.SolidBrush $accent), $tile)

    # Bogstavet laegges ind som en kurve og skaleres, saa det fylder det samme i alle
    # stoerrelser, og centreres i flisen.
    $letter = New-Object Drawing.Drawing2D.GraphicsPath
    $family = New-Object Drawing.FontFamily 'Segoe UI'
    $letter.AddString('s', $family, [int][Drawing.FontStyle]::Bold, 100, (New-Object Drawing.PointF 0, 0), [Drawing.StringFormat]::GenericTypographic)
    $box = $letter.GetBounds()
    $scale = ($size * 0.5) / $box.Height
    $m = New-Object Drawing.Drawing2D.Matrix
    $m.Translate(($size - $box.Width * $scale) / 2 - $box.X * $scale, ($size - $box.Height * $scale) / 2 - $box.Y * $scale)
    $m.Scale($scale, $scale)
    $letter.Transform($m)
    $g.FillPath((New-Object Drawing.SolidBrush $dark), $letter)

    $g.Dispose()
    return $bmp
}

# Smaa stoerrelser gemmes som klassisk 32-bit BMP (bredest understoettet), 256 px som PNG.
function Get-IconImageBytes([Drawing.Bitmap]$bmp) {
    $size = $bmp.Width
    $ms = New-Object IO.MemoryStream
    if ($size -ge 256) {
        $bmp.Save($ms, [Drawing.Imaging.ImageFormat]::Png)
        return ,$ms.ToArray()
    }
    $rect = New-Object Drawing.Rectangle 0, 0, $size, $size
    $data = $bmp.LockBits($rect, 'ReadOnly', 'Format32bppArgb')
    $pixels = New-Object byte[] ($size * $size * 4)
    for ($y = 0; $y -lt $size; $y++) {
        [Runtime.InteropServices.Marshal]::Copy([IntPtr]($data.Scan0.ToInt64() + $y * $data.Stride), $pixels, $y * $size * 4, $size * 4)
    }
    $bmp.UnlockBits($data)

    $maskStride = [int][Math]::Floor(($size + 31) / 32) * 4
    $w = New-Object IO.BinaryWriter $ms
    $w.Write([int]40); $w.Write([int]$size); $w.Write([int]($size * 2)); $w.Write([int16]1); $w.Write([int16]32)
    $w.Write([int]0); $w.Write([int]($size * $size * 4 + $maskStride * $size)); $w.Write([int]0); $w.Write([int]0); $w.Write([int]0); $w.Write([int]0)
    # Billedraekkerne ligger nedefra og op, foerst farver og derefter gennemsigtighedsmasken.
    for ($y = $size - 1; $y -ge 0; $y--) { $w.Write($pixels, $y * $size * 4, $size * 4) }
    for ($y = $size - 1; $y -ge 0; $y--) {
        $row = New-Object byte[] $maskStride
        for ($x = 0; $x -lt $size; $x++) {
            if ($pixels[($y * $size + $x) * 4 + 3] -eq 0) { $row[[Math]::Floor($x / 8)] = $row[[Math]::Floor($x / 8)] -bor (0x80 -shr ($x % 8)) }
        }
        $w.Write($row)
    }
    $w.Flush()
    return ,$ms.ToArray()
}

$bitmaps = $sizes | ForEach-Object { New-IconBitmap $_ }
$images = $bitmaps | ForEach-Object { ,(Get-IconImageBytes $_) }

$out = New-Object IO.MemoryStream
$w = New-Object IO.BinaryWriter $out
$w.Write([int16]0); $w.Write([int16]1); $w.Write([int16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $dim = if ($sizes[$i] -ge 256) { 0 } else { $sizes[$i] }
    $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([int16]1); $w.Write([int16]32); $w.Write([int]$images[$i].Length); $w.Write([int]$offset)
    $offset += $images[$i].Length
}
foreach ($image in $images) { $w.Write($image) }
$w.Flush()

$assets = Join-Path $repo 'assets'
New-Item -ItemType Directory -Force -Path $assets | Out-Null
[IO.File]::WriteAllBytes((Join-Path $assets 'sedit.ico'), $out.ToArray())
Write-Host "Skrev assets\sedit.ico ($($out.Length) bytes, $($sizes.Count) stoerrelser)"

if ($Preview) {
    # Oversigt paa lys og moerk baggrund, saa ikonet kan vurderes mod begge proceslinjer.
    $sheet = New-Object Drawing.Bitmap 760, 600
    $g = [Drawing.Graphics]::FromImage($sheet)
    $g.Clear([Drawing.Color]::FromArgb(243, 243, 243))
    $g.FillRectangle((New-Object Drawing.SolidBrush ([Drawing.Color]::FromArgb(32, 32, 32))), 0, 300, 760, 300)
    foreach ($top in 20, 320) {
        $x = 20
        foreach ($b in $bitmaps) { $g.DrawImageUnscaled($b, $x, $top); $x += $b.Width + 16 }
    }
    $g.Dispose()
    $sheet.Save($Preview, [Drawing.Imaging.ImageFormat]::Png)
}
