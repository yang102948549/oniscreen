# Draws the OniScreen icon source (icon-source.png, 1024x1024). Run `npm run icons` afterwards.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

function New-RoundRect([float]$x, [float]$y, [float]$w, [float]$h, [float]$r) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = $r * 2
    $p.AddArc($x, $y, $d, $d, 180, 90)
    $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    return $p
}

$size = 1024
$bmp = New-Object System.Drawing.Bitmap($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
$g.Clear([System.Drawing.Color]::Transparent)

# ONI toggle silhouette: blue pill, white thumb and a short white magnifier handle.
$g.ScaleTransform([single]($size / 22.0), [single]($size / 22.0))
$blueBrush = New-Object System.Drawing.SolidBrush([System.Drawing.ColorTranslator]::FromHtml('#007AFF'))
$whiteBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::White)
$handle = New-Object System.Drawing.Pen([System.Drawing.Color]::White, [single]1.65)
$handle.StartCap = $handle.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
$g.FillPath($blueBrush, (New-RoundRect 1.375 5.5 19.25 11 5.5))
$g.FillEllipse($whiteBrush, [single]9.503125, [single]6.003125, [single]8.59375, [single]8.59375)
$g.DrawLine($handle, [single]15.5, [single]12.9, [single]17.1, [single]15.6)
$blueBrush.Dispose(); $whiteBrush.Dispose(); $handle.Dispose()

$g.Dispose()
$out = Join-Path (Split-Path -Parent $PSScriptRoot) 'icon-source.png'
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
"saved $out"
