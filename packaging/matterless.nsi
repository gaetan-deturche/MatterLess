; The installer, and the other half of `matterless-view::update`.
;
; The updater downloads this, checks its signature, and runs it with
; `/P /UPDATE /R /ARGS`. Those four are not NSIS switches -- they were
; inventions of the Tauri template that used to build this, and installs
; already out there send them -- so they are read here by hand. `/S` is NSIS's
; own and is honoured too.
;
;   /P      passive: get on with it without asking anything
;   /UPDATE this is replacing a build, not arriving on a clean machine
;   /R      start the new build when finished, which is the half that makes an
;           update worth offering rather than merely downloading
;   /ARGS   what to start it with. The updater passes the switch with nothing
;           after it; anything there is handed to the app verbatim.
;
; Per user, under %LOCALAPPDATA%, deliberately: an update has to install
; without a UAC prompt, because the thing asking for it is a chat window that
; has just been told "yes" by somebody who wanted to keep reading.
;
; Built by the release workflow, and buildable by hand:
;
;   makensis /DVERSION=0.1.6 /DPAYLOAD=target\release\matterless-view.exe ^
;            /DOUTFILE=MatterLess-0.1.6-setup.exe packaging\matterless.nsi
;
; With /DFFMPEG=third_party\ffmpeg the video player's ffmpeg goes in beside the
; app, with its licence and the notice saying where its source is.

Unicode true

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"

!ifndef VERSION
  !error "VERSION is required: makensis /DVERSION=0.1.6 ..."
!endif
!ifndef PAYLOAD
  !error "PAYLOAD is required: the matterless-view.exe to install"
!endif
!ifndef OUTFILE
  !define OUTFILE "MatterLess-${VERSION}-setup.exe"
!endif

!define APP "MatterLess"
!define EXE "matterless-view.exe"
!define PUBLISHER "Gaetan Deturche"
; Where Windows lists what is installed. Under HKCU because this install is.
!define ARP "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"

Name "${APP}"
OutFile "${OUTFILE}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\${APP}"
; A second install goes where the first one did, which is what makes an update
; an update rather than a second copy.
InstallDirRegKey HKCU "Software\${APP}" "InstallDir"
SetCompressor /SOLID lzma
BrandingText "${APP} ${VERSION}"

; Four parts, because that is what Windows shows in the file's properties.
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "FileDescription" "${APP} setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "LegalCopyright" "MIT"

!define MUI_ICON "..\crates\matterless-view\resources\icons\icon.ico"
!define MUI_UNICON "..\crates\matterless-view\resources\icons\icon.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Open ${APP}"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

; Whether the updater sent us, and what it asked for afterwards.
Var Updating
Var Restart
Var Arguments

Function .onInit
  ${GetParameters} $R0

  ; Passive and update both mean "nobody is watching, get on with it". The
  ; reader already said yes, in the window this is about to replace.
  ClearErrors
  ${GetOptions} $R0 "/P" $R1
  ${IfNot} ${Errors}
    SetSilent silent
  ${EndIf}

  ClearErrors
  ${GetOptions} $R0 "/UPDATE" $R1
  ${IfNot} ${Errors}
    StrCpy $Updating 1
    SetSilent silent
  ${EndIf}

  ClearErrors
  ${GetOptions} $R0 "/R" $R1
  ${IfNot} ${Errors}
    StrCpy $Restart 1
  ${EndIf}

  ClearErrors
  ${GetOptions} $R0 "/ARGS" $R1
  ${IfNot} ${Errors}
    StrCpy $Arguments $R1
  ${EndIf}
FunctionEnd

; Waits for the build being replaced to let go of its own file.
;
; Not by killing it: the app launches this and then exits on purpose, so the
; file is released within a moment and a kill would only race that. Deleting
; the file is the test as well as the work -- it fails while the exe is still
; mapped, and succeeds the instant it is not.
;
; Half a minute, then on regardless: if it really is still held, `File` below
; fails loudly, which is better than an installer that hangs forever with no
; window to explain itself.
Function WaitForTheOldBuild
  StrCpy $0 0
  wait:
    ClearErrors
    Delete "$INSTDIR\${EXE}"
    ${IfNot} ${Errors}
      Return
    ${EndIf}
    IntOp $0 $0 + 1
    ${If} $0 >= 60
      DetailPrint "the running build is still holding its own file"
      Return
    ${EndIf}
    DetailPrint "waiting for the running build to close"
    Sleep 500
    Goto wait
FunctionEnd

Section "${APP}"
  SetOutPath "$INSTDIR"
  Call WaitForTheOldBuild
  File "/oname=${EXE}" "${PAYLOAD}"
!ifdef FFMPEG
  ; Separate files the app loads at run time, which is what the LGPL asks:
  ; anybody can put their own build of the same versions in their place.
  File "${FFMPEG}\bin\avutil-61.dll"
  File "${FFMPEG}\bin\swresample-7.dll"
  File "${FFMPEG}\bin\swscale-10.dll"
  File "${FFMPEG}\bin\avcodec-63.dll"
  File "${FFMPEG}\bin\avformat-63.dll"
  File "/oname=ffmpeg-LICENSE.txt" "${FFMPEG}\LICENSE.txt"
  File "/oname=ffmpeg-NOTICE.txt" "${__FILEDIR__}\ffmpeg-NOTICE.txt"
!if /FileExists "${FFMPEG}\dav1d-LICENSE.txt"
  File "${FFMPEG}\dav1d-LICENSE.txt"
!endif
!endif
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; The shortcut is not decoration: a toast is refused outright without a
  ; registered AppUserModelID, and on Windows that identity comes from a
  ; Start-menu shortcut. Rewritten on an update so a moved install does not
  ; leave one pointing at nothing.
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0

  WriteRegStr HKCU "Software\${APP}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\${APP}" "Version" "${VERSION}"

  WriteRegStr HKCU "${ARP}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${ARP}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${ARP}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${ARP}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${ARP}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${ARP}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
  WriteRegStr HKCU "${ARP}" "QuietUninstallString" "$\"$INSTDIR\uninstall.exe$\" /S"
  WriteRegDWORD HKCU "${ARP}" "NoModify" 1
  WriteRegDWORD HKCU "${ARP}" "NoRepair" 1
  ; In KiB, which is what the Programs list expects.
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${ARP}" "EstimatedSize" "$0"
SectionEnd

; Started last, and only when asked. `Exec` rather than `ExecShell` so the new
; build inherits nothing of this installer's elevation or environment.
Function .onInstSuccess
  ${If} $Restart == 1
    ${If} $Arguments == ""
      Exec "$\"$INSTDIR\${EXE}$\""
    ${Else}
      Exec "$\"$INSTDIR\${EXE}$\" $Arguments"
    ${EndIf}
  ${EndIf}
FunctionEnd

Section "Uninstall"
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\avutil-61.dll"
  Delete "$INSTDIR\swresample-7.dll"
  Delete "$INSTDIR\swscale-10.dll"
  Delete "$INSTDIR\avcodec-63.dll"
  Delete "$INSTDIR\avformat-63.dll"
  Delete "$INSTDIR\ffmpeg-LICENSE.txt"
  Delete "$INSTDIR\ffmpeg-NOTICE.txt"
  Delete "$INSTDIR\dav1d-LICENSE.txt"
  Delete "$INSTDIR\uninstall.exe"
  ; Only if it is empty: whatever else somebody put there is theirs.
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP}.lnk"
  DeleteRegKey HKCU "${ARP}"
  DeleteRegKey HKCU "Software\${APP}"
  ; The conversation store is deliberately left alone. It is the reader's
  ; messages, it cost a long first sync, and an uninstall is not a request to
  ; throw it away -- reinstalling finds it again.
SectionEnd
