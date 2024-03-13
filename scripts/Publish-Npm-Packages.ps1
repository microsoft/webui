$rootPath = Resolve-Path "./packages"
$directories = Get-ChildItem -Path $rootPath -Directory

Push-Location

foreach ($dir in $directories) {
    try {
        Set-Location -Path $dir.FullName -ErrorAction Stop
        Write-Host "Publishing package: $($dir.Name)"
        pnpm publish --access public
        Set-Location -Path $rootPath -ErrorAction Stop
    } catch {
        Write-Error "An error occurred while publishing the package: $($dir.Name)"
        Write-Error $_.Exception.Message
    } finally {
        # Return to the root path of your packages directory
        Set-Location -Path $rootPath
    }
}

Pop-Location
