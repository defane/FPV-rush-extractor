Unicode true
!include "MUI2.nsh"
Name "FPV Rush"
OutFile "${DIST}\FPV-Rush-${VERSION}-windows-x64-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\FPV Rush"
RequestExecutionLevel user
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Section "Install"
  SetOutPath "$INSTDIR"
  File /r "${STAGE}\*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\FPV Rush.lnk" "$INSTDIR\fpv-rush.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\FPVRush" "DisplayName" "FPV Rush"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\FPVRush" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\FPVRush" "UninstallString" '$"$INSTDIR\Uninstall.exe$"'
SectionEnd
Section "Uninstall"
  Delete "$SMPROGRAMS\FPV Rush.lnk"
  Delete "$INSTDIR\fpv-rush.exe"
  Delete "$INSTDIR\LICENSE-MIT"
  Delete "$INSTDIR\LICENSE-APACHE"
  RMDir /r "$INSTDIR\resources"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\FPVRush"
  ; The user's library, settings and recordings are intentionally preserved.
SectionEnd
