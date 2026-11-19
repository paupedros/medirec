# Descarga el modelo de voz Whisper (small, cuantizado q5_1, ~181 MB)
# a src-tauri/models/. Solo hace falta para compilar desde el código.
$ErrorActionPreference = "Stop"
$dest = Join-Path $PSScriptRoot "..\src-tauri\models\ggml-small-q5_1.bin"
$url  = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin"

if (Test-Path $dest) { Write-Host "El modelo ya existe: $dest"; exit 0 }
New-Item -ItemType Directory -Force (Split-Path $dest) | Out-Null
Write-Host "Descargando $url ..."
Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
Write-Host ("Listo: {0:N0} MB" -f ((Get-Item $dest).Length / 1MB))
