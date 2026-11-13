; Installer for Retrograde 2.x, the native (C# + Avalonia) build.
; Built by installer\build.ps1, which passes VERSION, SRC (the publish folder) and OUTFILE.
;
; Installs per user (no admin prompt) to %LOCALAPPDATA%\Programs\Retrograde, the folder the Electron
; build (1.x) used, which 2.0 replaces:
; - the native 1.x previews ("Retrograde Native", own folder) are removed: folder, shortcuts, uninstall entry;
; - if the Electron build is still installed there, setup stops and asks for it to be uninstalled first,
;   rather than running its uninstaller (which closes every process called Retrograde.exe).
; The library in %USERPROFILE%\.retrograde is the user's data: neither setup nor the uninstaller touches it.

Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!ifndef VERSION
  !error "pass /DVERSION=x.y.z"
!endif
!ifndef SRC
  !error "pass /DSRC=<publish folder>"
!endif
!ifndef OUTFILE
  !define OUTFILE "Retrograde Setup ${VERSION}.exe"
!endif

!define APPNAME "Retrograde"
!define EXENAME "Retrograde.exe"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Retrograde"
; The native 1.x previews, removed on install.
!define OLDNAME "Retrograde Native"
!define OLDKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\RetrogradeNative"
; What the Electron build leaves in its folder.
!define ELECTRON_UNINSTALLER "Uninstall Retrograde.exe"
!define ICON "${__FILEDIR__}\..\..\build\icon.ico"

Name "${APPNAME}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APPNAME}"
InstallDirRegKey HKCU "${UNINSTKEY}" "InstallLocation"
BrandingText "${APPNAME} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APPNAME}"
VIAddVersionKey "FileDescription" "${APPNAME} installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "Copyright © 2025"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_NODESC
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXENAME}"
!define MUI_FINISHPAGE_RUN_TEXT "Launch ${APPNAME}"

!define MUI_PAGE_CUSTOMFUNCTION_LEAVE CheckElectron
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; A running copy holds Retrograde.exe open. Ask the user to close it instead of killing processes by
; name: the Electron build's process is also called Retrograde.exe.
!macro WaitForCloseFunc PREFIX
  Function ${PREFIX}WaitForClose
    ${IfNot} ${FileExists} "$INSTDIR\${EXENAME}"
      Return
    ${EndIf}
    retry:
    ClearErrors
    Delete "$INSTDIR\${EXENAME}"
    ${If} ${Errors}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${APPNAME} is running. Close it (tray icon > Exit), then press Retry." /SD IDCANCEL IDRETRY retry
      Abort
    ${EndIf}
  FunctionEnd
!macroend
!insertmacro WaitForCloseFunc ""
!insertmacro WaitForCloseFunc "un."

; The Electron build in the chosen folder: its files would mix with these. Checked when leaving the
; folder page, so a different folder can still be picked, and again before installing (silent runs).
Function CheckElectron
  ${If} ${FileExists} "$INSTDIR\${ELECTRON_UNINSTALLER}"
    MessageBox MB_OK|MB_ICONEXCLAMATION "The Electron build of Retrograde (1.x) is installed in$\r$\n$INSTDIR.$\r$\n$\r$\nRetrograde ${VERSION} replaces it. Uninstall it first (Windows Settings > Apps > Retrograde), then run this setup again.$\r$\n$\r$\nYour library, playlists and favorites in %USERPROFILE%\.retrograde stay." /SD IDOK
    Abort
  ${EndIf}
FunctionEnd

; Native 1.x previews were installed as "Retrograde Native" in their own folder.
Function RemoveOldNative
  StrCpy $0 "$LOCALAPPDATA\Programs\${OLDNAME}"
  ${If} $0 != $INSTDIR
  ${AndIf} ${FileExists} "$0\${EXENAME}"
    retry:
    ClearErrors
    Delete "$0\${EXENAME}"
    ${If} ${Errors}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${OLDNAME} is running. Close it (tray icon > Exit), then press Retry." /SD IDCANCEL IDRETRY retry
      Abort
    ${EndIf}
  ${EndIf}
  ${If} $0 != $INSTDIR
    RMDir /r "$0"
  ${EndIf}
  Delete "$SMPROGRAMS\${OLDNAME}.lnk"
  Delete "$DESKTOP\${OLDNAME}.lnk"
  DeleteRegKey HKCU "${OLDKEY}"
FunctionEnd

Section "${APPNAME}" SecMain
  SectionIn RO
  Call CheckElectron
  SetOutPath "$INSTDIR"
  Call WaitForClose
  Call RemoveOldNative
  ; Files of an older version that this one no longer ships would otherwise stay behind.
  RMDir /r "$INSTDIR\bin"
  Delete "$INSTDIR\*.dll"
  File /r "${SRC}\*.*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateShortCut "$SMPROGRAMS\${APPNAME}.lnk" "$INSTDIR\${EXENAME}" "" "$INSTDIR\${EXENAME}" 0

  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "Retrograde"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\${EXENAME}"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINSTKEY}" "EstimatedSize" $0
SectionEnd

Section "Desktop shortcut" SecDesktop
  CreateShortCut "$DESKTOP\${APPNAME}.lnk" "$INSTDIR\${EXENAME}" "" "$INSTDIR\${EXENAME}" 0
SectionEnd

Section "Uninstall"
  Call un.WaitForClose
  Delete "$SMPROGRAMS\${APPNAME}.lnk"
  Delete "$DESKTOP\${APPNAME}.lnk"
  DeleteRegKey HKCU "${UNINSTKEY}"
  ; Only the program folder. The library, covers, playlists and settings in %USERPROFILE%\.retrograde stay.
  RMDir /r "$INSTDIR"
SectionEnd
