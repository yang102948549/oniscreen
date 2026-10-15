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

# Minimal screen and pen mark, matching ui/assets/oni-screen.svg.
$g.ScaleTransform([single]($size / 24.0), [single]($size / 24.0))
$blueBrush = New-Object System.Drawing.SolidBrush([System.Drawing.ColorTranslator]::FromHtml('#2383E2'))
$whiteBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::White)
$g.FillPath($blueBrush, (New-RoundRect 0 0 24 24 5.5))
$line = New-Object System.Drawing.Pen([System.Drawing.Color]::White, [single]1.5)
$line.StartCap = $line.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
$g.DrawPath($line, (New-RoundRect 4.5 5.5 15 11 1.5))
$g.DrawLine($line, [single]12, [single]16.5, [single]12, [single]19)
$g.DrawLine($line, [single]9, [single]19, [single]15, [single]19)
$points = [System.Drawing.PointF[]]@([System.Drawing.PointF]::new(9,13.5), [System.Drawing.PointF]::new(9.7,10.8), [System.Drawing.PointF]::new(14,6.5), [System.Drawing.PointF]::new(16,8.5), [System.Drawing.PointF]::new(11.7,12.8))
$g.FillPolygon($whiteBrush, $points)
$blueBrush.Dispose(); $whiteBrush.Dispose(); $line.Dispose()

$g.Dispose()
$out = Join-Path (Split-Path -Parent $PSScriptRoot) 'icon-source.png'
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
"saved $out"
