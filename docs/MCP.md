# Serveur MCP (à implémenter — milestone M5)

`mosswake-mcp` est un serveur MCP **stdio** qui permet à un agent IA de créer et modifier des niveaux.

Tools prévus : `list_levels`, `create_level`, `get_level`, `set_tiles`, `place_entity`, `remove_entity`,
`set_rules`, `write_script`, `validate_level`, `simulate_level`, `render_preview`.

Configuration Claude Code (`.mcp.json` à la racine) :
```json
{ "mcpServers": { "mosswake": { "command": "cargo", "args": ["run", "-q", "-p", "mosswake-mcp"] } } }
```
