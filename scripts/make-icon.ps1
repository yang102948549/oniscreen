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

$blue = [System.Drawing.ColorTranslator]::FromHtml('#1B70C9')
$white = [System.Drawing.Color]::White

# Background: Oni action-blue gradient
$bg = New-RoundRect 0 0 $size $size 228
$grad = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
    (New-Object System.Drawing.PointF(0, 0)), (New-Object System.Drawing.PointF(0, $size)),
    [System.Drawing.ColorTranslator]::FromHtml('#2E8AE6'), [System.Drawing.ColorTranslator]::FromHtml('#155FB0'))
$g.FillPath($grad, $bg)

# Screen (monitor) and stand
$wb = New-Object System.Drawing.SolidBrush($white)
$g.FillPath($wb, (New-RoundRect 172 228 680 448 60))
$g.FillRectangle($wb, 468, 670, 88, 90)
$g.FillPath($wb, (New-RoundRect 344 748 336 60 30))

# Magnifier inside the screen
$g.FillEllipse((New-Object System.Drawing.SolidBrush([System.Drawing.ColorTranslator]::FromHtml('#DCEBFB'))), 358, 318, 236, 236)
$pen = New-Object System.Drawing.Pen($blue, 54)
$pen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
$pen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
$g.DrawEllipse($pen, 358, 318, 236, 236)
$g.DrawLine($pen, 566, 526, 652, 612)

$g.Dispose()
$out = Join-Path (Split-Path -Parent $PSScriptRoot) 'icon-source.png'
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
"saved $out"
