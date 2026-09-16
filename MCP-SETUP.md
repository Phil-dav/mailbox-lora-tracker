# Outils MCP (Context7, Firecrawl, KiCad)

Ce projet utilise 3 serveurs MCP pour étendre les capacités de Claude : Context7 (documentation de bibliothèques à jour), Firecrawl (recherche/lecture web, contourne certains blocages anti-robot), et un serveur KiCad (analyse de schémas/PCB, ERC/DRC, traçage de connexions).

La configuration (`.mcp.json`) est synchronisée par Dropbox et arrive donc automatiquement sur toutes les machines. Mais certains éléments sont propres à chaque ordinateur et doivent être réinstallés une fois par machine :
- Node.js (pour Context7 et Firecrawl)
- le cache npx (pour éviter des timeouts au premier lancement)
- les paquets Python installés dans le Python de KiCad (pour le serveur KiCad)

## Installation sur une nouvelle machine

1. Ouvrir un PowerShell à la racine de ce dossier ("Boite aux lettres").
2. Lancer :
   ```
   .\setup-mcp-tools.ps1
   ```
   Si Node.js vient d'être installé, le script s'arrête et demande de rouvrir un nouveau PowerShell puis de le relancer (le temps que le PATH se mette à jour).
3. Redémarrer VS Code.
4. Vérifier que les 3 serveurs (`context7`, `firecrawl`, `kicad`) apparaissent connectés.

## Détails / dépannage par outil

- **KiCad** : voir `../kicad-mcp-server/NOTES-INSTALLATION.md`. Si la version de KiCad diffère de 10.0, corriger le chemin dans `.mcp.json` (le script affiche le bon chemin).
- **Firecrawl** : la clé `FIRECRAWL_API_KEY` doit être présente dans `.mcp.json` (normalement déjà synchronisée par Dropbox — ne pas la recréer). Si erreur `ERR_MODULE_NOT_FOUND` au démarrage : cache npx corrompu, supprimer le dossier concerné dans `%LOCALAPPDATA%\npm-cache\_npx\` et relancer `npx -y firecrawl-mcp --help`.
- **Context7** : pas de clé nécessaire. Si timeout au premier lancement, relancer simplement `npx -y @upstash/context7-mcp@latest --help` une fois pour terminer le téléchargement.

`.mcp.json` contient une clé API (Firecrawl) — il est volontairement exclu de git (`.gitignore`), donc il ne se synchronise que par Dropbox, jamais par un `git push`.
