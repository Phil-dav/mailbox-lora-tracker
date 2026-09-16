# Configure les 3 serveurs MCP (Context7, Firecrawl, KiCad) sur CETTE machine.
# .mcp.json est deja synchronise par Dropbox et contient deja la configuration ;
# ce script installe seulement ce qui est propre a chaque ordinateur
# (Node.js, cache npx, paquets Python de KiCad).
#
# A lancer une fois par nouvelle machine, puis redemarrer VS Code.

Write-Host "=== 1. Node.js (necessaire pour Context7 et Firecrawl) ==="
$node = Get-Command node -ErrorAction SilentlyContinue
if (-not $node) {
    Write-Host "Node.js introuvable. Installation via winget..."
    winget install --id OpenJS.NodeJS.LTS --silent --accept-package-agreements --accept-source-agreements
    Write-Host ""
    Write-Host "IMPORTANT : ferme et rouvre ce PowerShell (pour recharger le PATH), puis relance ce script."
    exit 0
} else {
    Write-Host "Node.js deja present : $($node.Source)"
}

Write-Host ""
Write-Host "=== 2. Prechauffage du cache npx (evite les timeouts de connexion au premier lancement) ==="
npx -y @upstash/context7-mcp@latest --help
npx -y firecrawl-mcp --help

Write-Host ""
Write-Host "=== 3. kicad-mcp-server (paquets Python dans le Python de KiCad) ==="
$kicadScript = Join-Path $PSScriptRoot "..\kicad-mcp-server\install-kicad-python.ps1"
if (Test-Path $kicadScript) {
    & $kicadScript
} else {
    Write-Host "Script introuvable a $kicadScript"
    Write-Host "Verifie que Dropbox a bien fini de synchroniser le dossier 'kicad-mcp-server'."
}

Write-Host ""
Write-Host "=== Termine ==="
Write-Host "Verifie que la cle FIRECRAWL_API_KEY est bien presente dans .mcp.json (normalement deja synchronisee par Dropbox)."
Write-Host "Redemarre VS Code pour que les 3 serveurs MCP (context7, firecrawl, kicad) se connectent."
