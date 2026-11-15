# craft.config.psd1 - per-repo settings for start.ps1 and scripts/release.ps1.
@{
    Name        = 'stockcraft'
    Display     = 'StockCraft'
    AppPackage  = 'stockcraft-app'
    AppBin      = 'stockcraft-app'
    AppSmoke    = @('--smoke')
    CliPackage  = 'stockcraft-mcp'
    CliBin      = 'stockcraft'
    CliSmoke    = @('--db', '{db}', 'status')
    McpArgs     = @('mcp')
    McpSmoke    = @('mcp')
    Installer   = $false
    Mcpb        = $false
}
