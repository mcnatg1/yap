' Runs a PowerShell install script with no visible window. Args: <script.ps1> <InstallDir>
Option Explicit

If WScript.Arguments.Count < 2 Then
  WScript.Quit 1
End If

Dim scriptPath, installDir, psCmd, shell, exitCode
scriptPath = WScript.Arguments(0)
installDir = WScript.Arguments(1)

psCmd = "powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File """ _
  & scriptPath & """ -InstallDir """ & installDir & """ -IfNeeded"

Set shell = CreateObject("WScript.Shell")
exitCode = shell.Run(psCmd, 0, True)
WScript.Quit exitCode
