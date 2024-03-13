$rootPath = Resolve-Path "./packages"
$directories = Get-ChildItem -Path $rootPath -Directory

Push-Location

foreach ($dir in $directories) {
    try {
        Set-Location -Path $dir.FullName -ErrorAction Stop
        Write-Host "Checking package in directory: $($dir.Name)"
        
        # Get the package name and version from package.json
        $packageJson = Get-Content -Path "./package.json" | ConvertFrom-Json
        $packageName = $packageJson.name
        $localVersion = $packageJson.version

        # Get the npm version
        $npmVersion = ""
        try {
            $npmVersion = (npm show $packageName version) -replace "`n", ""
        } catch {
            Write-Host "Package $packageName not found on npm"
        }

        if ($localVersion -ne $npmVersion) {
            Write-Host "Publishing package: $packageName"
            pnpm publish --access public
        } else {
            Write-Host "Skipping package: $packageName, version has not changed"
        }

        Set-Location -Path $rootPath -ErrorAction Stop
    } catch {
        Write-Error "An error occurred while publishing the package: $packageName"
        Write-Error $_.Exception.Message
    } finally {
        # Return to the root path of your packages directory
        Set-Location -Path $rootPath
    }
}

Pop-Location