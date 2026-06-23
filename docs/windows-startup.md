# Windows Startup Setup

Last updated: 2026-06-23

DevRelay does not install a Windows per-user background service in M2 dev mode.
Until packaged Windows support exists, run the agent explicitly from a terminal
or wire it into a user startup mechanism you control.

This is a development workaround, not production Windows support. Windows named
pipe IPC and per-user pipe ACL are still required before the desktop UI can use
the Windows agent as its state authority.

Manual dev command:

```powershell
$env:DEVRELAY_HOME="$env:LOCALAPPDATA\DevRelay"
devrelay-agent --foreground --log-level info
```

For local startup, create a shortcut or scheduled task that runs the same
command at user logon. Keep it per-user, avoid elevated privileges, and keep
`DEVRELAY_HOME` under the current user's profile.

Do not point Windows native tools and WSL tools at the same working tree.
Register separate workspaces for Windows native and each WSL distro.
