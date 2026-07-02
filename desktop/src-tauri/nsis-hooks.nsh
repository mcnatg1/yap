!macro NSIS_HOOK_POSTINSTALL
  !searchreplace YAP_MAINBINARYDIR "${MAINBINARYSRCPATH}" "${MAINBINARYNAME}.exe" ""
  File "/oname=$INSTDIR\WebView2Loader.dll" "${YAP_MAINBINARYDIR}WebView2Loader.dll"
  File /nonfatal "/oname=$INSTDIR\libunwind.dll" "${YAP_MAINBINARYDIR}libunwind.dll"

  DetailPrint "Downloading transcription engine and model (SHA-256 verified)..."
  ExecWait 'wscript.exe //B //Nologo "$INSTDIR\_up_\scripts\run-hidden.vbs" "$INSTDIR\_up_\scripts\install-all.ps1" "$INSTDIR"' $R0
  IntCmp $R0 0 install_ok install_fail install_fail
  install_fail:
    MessageBox MB_OK|MB_ICONEXCLAMATION "Yap could not download the transcription engine or model.$\nCheck your network connection and run the installer again."
    Goto install_done
  install_ok:
    DetailPrint "Transcription engine and model installed."
  install_done:
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\libunwind.dll"
  Delete "$INSTDIR\crispasr.exe"
!macroend
