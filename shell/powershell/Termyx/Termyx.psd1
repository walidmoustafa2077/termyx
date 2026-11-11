@{
    RootModule        = 'Termyx.psm1'
    ModuleVersion     = '0.1.0'
    GUID              = '0f8f5d4c-3f4c-4a71-9b0e-6a2f3c9d1e57'
    Author            = 'Termyx'
    Description       = 'Command-aware autocorrect and spellcheck for Windows PowerShell 5.1.'
    PowerShellVersion = '5.1'
    FunctionsToExport = @(
        'Enable-TermyxAutocorrect',
        'Disable-TermyxAutocorrect',
        'Get-TermyxStatus',
        'Add-TermyxWord',
        'Update-TermyxCommandCache',
        'Set-TermyxOption'
    )
    CmdletsToExport   = @()
    VariablesToExport = @()
    AliasesToExport   = @()
    PrivateData       = @{
        PSData = @{
            Tags = @('autocorrect', 'spellcheck', 'psreadline')
        }
    }
}
