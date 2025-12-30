# NomadServer Docker Push Script
# Pushes the tagged image to Docker Hub

Write-Host "=== NomadServer Docker Push Script ===" -ForegroundColor Cyan
Write-Host ""

# Step 0: Check if Docker is running
Write-Host "[0/2] Checking Docker daemon..." -ForegroundColor Yellow
$dockerCheck = docker info 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Host "ERROR: Docker is not running!" -ForegroundColor Red
    Write-Host "Please start Docker Desktop and try again." -ForegroundColor Yellow
    exit 1
}
Write-Host "[OK] Docker is running" -ForegroundColor Green
Write-Host ""

# Check if image exists
Write-Host "[1/2] Checking if image exists..." -ForegroundColor Yellow
$imageExists = docker images zenderady/nomad-server:latest -q
if (-not $imageExists) {
    Write-Host "ERROR: Image zenderady/nomad-server:latest not found!" -ForegroundColor Red
    Write-Host "Please build the image first: .\build-docker.ps1" -ForegroundColor Yellow
    exit 1
}
Write-Host "[OK] Image found" -ForegroundColor Green
Write-Host ""

# Push to Docker Hub
Write-Host "[2/2] Pushing to Docker Hub..." -ForegroundColor Yellow
docker push zenderady/nomad-server:latest
if ($LASTEXITCODE -ne 0) {
    Write-Host "ERROR: Docker push failed!" -ForegroundColor Red
    Write-Host "Make sure you're logged in: docker login" -ForegroundColor Yellow
    exit 1
}
Write-Host "[OK] Image pushed successfully!" -ForegroundColor Green
Write-Host ""

Write-Host "=== Push Complete ===" -ForegroundColor Cyan
Write-Host "Image is now available on Docker Hub!" -ForegroundColor Cyan
Write-Host ""
