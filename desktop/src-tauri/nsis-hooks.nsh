!macro NSIS_HOOK_POSTINSTALL
  !searchreplace YAP_MAINBINARYDIR "${MAINBINARYSRCPATH}" "${MAINBINARYNAME}.exe" ""
  File "/oname=$INSTDIR\WebView2Loader.dll" "${YAP_MAINBINARYDIR}WebView2Loader.dll"
  File /nonfatal "/oname=$INSTDIR\libunwind.dll" "${YAP_MAINBINARYDIR}libunwind.dll"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\libunwind.dll"
!macroend
