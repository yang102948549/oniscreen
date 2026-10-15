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

# Transparent chalkboard and magnifier, matching ui/assets/oni-screen.svg.
$g.ScaleTransform([single]($size / 24.0), [single]($size / 24.0))
$boardBrush = New-Object System.Drawing.SolidBrush([System.Drawing.ColorTranslator]::FromHtml('#292927'))
$whiteBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::White)
$boardLine = New-Object System.Drawing.Pen($boardBrush, [single]1.6)
$boardLine.StartCap = $boardLine.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
$blueLine = New-Object System.Drawing.Pen([System.Drawing.ColorTranslator]::FromHtml('#2383E2'), [single]2)
$blueLine.StartCap = $blueLine.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
$g.FillPath($boardBrush, (New-RoundRect 2 3 17 12 1.5))
$g.DrawLine($boardLine, [single]10.5, [single]15, [single]10.5, [single]19)
$g.DrawLine($boardLine, [single]6, [single]19, [single]14, [single]19)
$g.FillEllipse($whiteBrush, [single]11, [single]9, [single]8, [single]8)
$g.DrawEllipse($blueLine, [single]11, [single]9, [single]8, [single]8)
$g.DrawLine($blueLine, [single]18, [single]16, [single]22, [single]20)
$boardBrush.Dispose(); $whiteBrush.Dispose(); $boardLine.Dispose(); $blueLine.Dispose()

$g.Dispose()
$out = Join-Path (Split-Path -Parent $PSScriptRoot) 'icon-source.png'
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
"saved $out"
