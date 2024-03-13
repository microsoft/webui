$rootPath = "./packages"
$directories = Get-ChildItem -Path $rootPath -Directory

Push-Location

foreach ($dir in $directories) {
    Set-Location -Path $dir.FullName
    Write-Host "Publishing package: $($dir.Name)"
    pnpm publish --access public
    Set-Location -Path $rootPath
}

Pop-Location
