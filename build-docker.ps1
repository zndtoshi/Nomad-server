# NomadServer Docker Build Script
# Builds and tags the Docker image for deployment

Write-Host "=== NomadServer Docker Build Script ===" -ForegroundColor Cyan
Write-Host ""

# Step 0: Check if Docker is running
Write-Host "[0/4] Checking Docker daemon..." -ForegroundColor Yellow
$dockerCheck = docker info 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Host "ERROR: Docker is not running!" -ForegroundColor Red
    Write-Host "Please start Docker Desktop and try again." -ForegroundColor Yellow
    exit 1
}
Write-Host "[OK] Docker is running" -ForegroundColor Green
Write-Host ""

# Step 1: Build the Docker image
Write-Host "[1/4] Building Docker image..." -ForegroundColor Yellow
docker build -t nomad-server:latest .
if ($LASTEXITCODE -ne 0) {
    Write-Host "ERROR: Docker build failed!" -ForegroundColor Red
    exit 1
}
Write-Host "[OK] Docker image built successfully" -ForegroundColor Green
Write-Host ""

# Step 2: Tag for Docker Hub
Write-Host "[2/4] Tagging image for Docker Hub..." -ForegroundColor Yellow
docker tag nomad-server:latest zenderady/nomad-server:latest
if ($LASTEXITCODE -ne 0) {
    Write-Host "ERROR: Docker tag failed!" -ForegroundColor Red
    exit 1
}
Write-Host "[OK] Image tagged as zenderady/nomad-server:latest" -ForegroundColor Green
Write-Host ""

# Step 3: Push to Docker Hub (only if build and tag succeeded)
Write-Host "[3/4] Pushing to Docker Hub..." -ForegroundColor Yellow
docker push zenderady/nomad-server:latest
if ($LASTEXITCODE -ne 0) {
    Write-Host "ERROR: Docker push failed!" -ForegroundColor Red
    Write-Host "Make sure you're logged in: docker login" -ForegroundColor Yellow
    exit 1
}
Write-Host "[OK] Image pushed successfully!" -ForegroundColor Green
Write-Host ""

# Step 4: Show image info
Write-Host "[4/4] Image information:" -ForegroundColor Yellow
docker images nomad-server:latest
docker images zenderady/nomad-server:latest
Write-Host ""

Write-Host "=== Build and Push Complete ===" -ForegroundColor Cyan
Write-Host "Image built, tagged, and pushed successfully!" -ForegroundColor Cyan
Write-Host ""
