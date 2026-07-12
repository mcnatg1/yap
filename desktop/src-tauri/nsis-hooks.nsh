!include "TextFunc.nsh"
!include "FileFunc.nsh"

Var YapAutomatedDelete
Var YapDeleteValidationFailure
Var YapDeleteToken

!macro YAP_ABORT_DELETE_WITH_ROLLBACK MESSAGE
  ClearErrors
  Rename "$LOCALAPPDATA\${PRODUCTNAME}.delete-quarantine" "$LOCALAPPDATA\${PRODUCTNAME}"
  ${If} ${Errors}
    SetErrorLevel 87
    Abort "${MESSAGE} Recovery data remains in $LOCALAPPDATA\${PRODUCTNAME}.delete-quarantine."
  ${EndIf}
  SetErrorLevel 87
  Abort "${MESSAGE}"
!macroend

Function un.YapValidateDeleteEntry
  ClearErrors
  ${un.GetFileAttributes} "$R9" "REPARSE_POINT" $R6
  ${If} ${Errors}
    StrCpy $YapDeleteValidationFailure "attribute"
    Push "StopLocate"
    Return
  ${EndIf}
  ${If} $R6 == "1"
    StrCpy $YapDeleteValidationFailure "reparse"
    Push "StopLocate"
    Return
  ${EndIf}
  Push ""
FunctionEnd

!macro YAP_VALIDATE_DELETE_TREE PATH LABEL
  ${If} ${FileExists} "${PATH}"
    ClearErrors
    ${un.GetFileAttributes} "${PATH}" "REPARSE_POINT" $R6
    ${If} ${Errors}
      SetErrorLevel 87
      Abort "${LABEL} could not validate its root."
    ${EndIf}
    ${If} $R6 == "1"
      SetErrorLevel 87
      Abort "${LABEL} refuses a reparse root."
    ${EndIf}
    StrCpy $YapDeleteValidationFailure ""
    ClearErrors
    ${un.Locate} "${PATH}" "/L=FD /M=*.* /S=0B /G=1" "un.YapValidateDeleteEntry"
    ${If} ${Errors}
      SetErrorLevel 87
      Abort "${LABEL} could not inspect its tree."
    ${EndIf}
    ${If} $YapDeleteValidationFailure != ""
      SetErrorLevel 87
      Abort "${LABEL} refuses a nested reparse point or unreadable entry."
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  StrCpy $YapAutomatedDelete "0"
  StrCpy $YapDeleteToken ""
  ${GetOptions} $CMDLINE "/DELETEAPPDATA=" $YapDeleteToken
  ${IfNot} ${Errors}
    ${If} $YapDeleteToken == ""
      SetErrorLevel 87
      Abort "Automated application-data deletion requires a non-empty run token."
    ${EndIf}

    StrCpy $R8 "0"
    ${If} "${PRODUCTNAME}" == "Yap.Test"
      StrCpy $R8 "1"
    ${Else}
      ReadEnvStr $R3 "RUNNER_ENVIRONMENT"
      ${If} $R3 == "github-hosted"
        StrCpy $R8 "1"
      ${Else}
        ReadEnvStr $R4 "USERNAME"
        ${If} $R4 == "WDAGUtilityAccount"
          StrCpy $R8 "1"
        ${Else}
          StrCpy $R6 $R4 7
          StrCpy $R7 $R4 8
          ${If} $R6 == "YapTest"
          ${OrIf} $R7 == "YapSmoke"
            ReadEnvStr $R5 "USERPROFILE"
            ClearErrors
            FileOpen $R1 "$R5\.yap-disposable-test-profile" r
            ${IfNot} ${Errors}
              FileRead $R1 $R2
              FileClose $R1
              ${TrimNewLines} $R2 $R2
              ${If} $R2 == "yap-disposable-profile-v1"
                StrCpy $R8 "1"
              ${EndIf}
            ${EndIf}
          ${EndIf}
        ${EndIf}
      ${EndIf}
    ${EndIf}
    ${If} $R8 != "1"
      SetErrorLevel 87
      Abort "Automated production-data deletion requires an isolated test environment."
    ${EndIf}

    ClearErrors
    FileOpen $R1 "$LOCALAPPDATA\${PRODUCTNAME}\.yap-destructive-uninstall-test" r
    ${If} ${Errors}
      SetErrorLevel 87
      Abort "Automated application-data deletion requires the isolated-test sentinel."
    ${EndIf}
    FileRead $R1 $R2
    FileClose $R1
    ${TrimNewLines} $R2 $R2
    ${If} $R2 != $YapDeleteToken
      SetErrorLevel 87
      Abort "The application-data deletion token does not match this test run."
    ${EndIf}

    StrCpy $YapAutomatedDelete "1"
    StrCpy $DeleteAppDataCheckboxState 1
  ${EndIf}
  ${If} $DeleteAppDataCheckboxState == 1
    !insertmacro YAP_VALIDATE_DELETE_TREE "$APPDATA\${BUNDLEID}" "Roaming application-data deletion"
    !insertmacro YAP_VALIDATE_DELETE_TREE "$LOCALAPPDATA\${BUNDLEID}" "Legacy local application-data deletion"
  ${EndIf}
  ClearErrors
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState == 1
    ${If} $YapAutomatedDelete == "1"
      ClearErrors
      FileOpen $R1 "$LOCALAPPDATA\${PRODUCTNAME}\.yap-destructive-uninstall-test" r
      ${If} ${Errors}
        SetErrorLevel 87
        Abort "Automated application-data deletion lost its isolated-test sentinel."
      ${EndIf}
      FileRead $R1 $R2
      FileClose $R1
      ${TrimNewLines} $R2 $R2
      ${If} $R2 != $YapDeleteToken
        SetErrorLevel 87
        Abort "The application-data deletion token changed before final deletion."
      ${EndIf}
    ${EndIf}
    ${If} ${FileExists} "$LOCALAPPDATA\${PRODUCTNAME}.delete-quarantine"
      SetErrorLevel 87
      Abort "Application-data deletion found recovery data in the fixed quarantine."
    ${EndIf}
    ${If} ${FileExists} "$LOCALAPPDATA\${PRODUCTNAME}"
      ClearErrors
      Rename "$LOCALAPPDATA\${PRODUCTNAME}" "$LOCALAPPDATA\${PRODUCTNAME}.delete-quarantine"
      ${If} ${Errors}
        SetErrorLevel 87
        Abort "Application-data deletion could not quarantine its tree."
      ${EndIf}
      ClearErrors
      ${un.GetFileAttributes} "$LOCALAPPDATA\${PRODUCTNAME}.delete-quarantine" "REPARSE_POINT" $R6
      ${If} ${Errors}
        !insertmacro YAP_ABORT_DELETE_WITH_ROLLBACK "Application-data deletion could not revalidate the quarantine."
      ${EndIf}
      ${If} $R6 == "1"
        !insertmacro YAP_ABORT_DELETE_WITH_ROLLBACK "Application-data deletion refuses a reparse quarantine."
      ${EndIf}
      StrCpy $YapDeleteValidationFailure ""
      ClearErrors
      ${un.Locate} "$LOCALAPPDATA\${PRODUCTNAME}.delete-quarantine" "/L=FD /M=*.* /S=0B /G=1" "un.YapValidateDeleteEntry"
      ${If} ${Errors}
        !insertmacro YAP_ABORT_DELETE_WITH_ROLLBACK "Application-data deletion could not inspect the quarantined tree."
      ${EndIf}
      ${If} $YapDeleteValidationFailure != ""
        !insertmacro YAP_ABORT_DELETE_WITH_ROLLBACK "Application-data deletion refuses a nested reparse point or unreadable entry."
      ${EndIf}
      ${If} $YapAutomatedDelete == "1"
        ClearErrors
        FileOpen $R1 "$LOCALAPPDATA\${PRODUCTNAME}.delete-quarantine\.yap-destructive-uninstall-test" r
        ${If} ${Errors}
          !insertmacro YAP_ABORT_DELETE_WITH_ROLLBACK "Automated application-data deletion lost its sentinel after quarantine."
        ${EndIf}
        FileRead $R1 $R2
        FileClose $R1
        ${TrimNewLines} $R2 $R2
        ${If} $R2 != $YapDeleteToken
          !insertmacro YAP_ABORT_DELETE_WITH_ROLLBACK "The quarantined application-data token does not match this test run."
        ${EndIf}
      ${EndIf}
      RMDir /r "$LOCALAPPDATA\${PRODUCTNAME}.delete-quarantine"
      ${If} ${FileExists} "$LOCALAPPDATA\${PRODUCTNAME}.delete-quarantine"
        !insertmacro YAP_ABORT_DELETE_WITH_ROLLBACK "Application-data deletion did not completely remove its quarantine."
      ${EndIf}
    ${EndIf}
  ${EndIf}
!macroend
