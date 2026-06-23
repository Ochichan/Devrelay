# Windows Startup Setup

DevRelay does not install a Windows per-user background service in M2 dev mode.
Until packaged Windows support exists, run the agent explicitly from a terminal
or wire it into a user startup mechanism you control.

Manual dev command:

```powershell
$env:DEVRELAY_HOME="$env:LOCALAPPDATA\DevRelay"
devrelay-agent --foreground --log-level info
```

For local startup, create a shortcut or scheduled task that runs the same
command at user logon. Keep it per-user, avoid elevated privileges, and keep
`DEVRELAY_HOME` under the current user's profile.
