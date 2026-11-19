@echo off
title Cai CLI cho Room Control AI
echo ==================================================
echo   Cai CLI cho Room Control AI
echo   claude (Anthropic) - codex (OpenAI) - gemini (Google)
echo ==================================================
echo.

where npm >nul 2>nul
if errorlevel 1 (
  echo [!] May chua co Node.js / npm. Dang cai Node.js LTS bang winget...
  winget install -e --id OpenJS.NodeJS.LTS --accept-package-agreements --accept-source-agreements
  echo.
  echo Cai Node.js xong. DONG cua so nay roi bam dup lai cai-cli.bat de cai tiep.
  pause
  exit /b
)

echo [1/2] Dang cai 3 CLI bang npm (mat vai phut)...
call npm install -g @anthropic-ai/claude-code @openai/codex @google/gemini-cli
if errorlevel 1 (
  echo.
  echo [!] npm cai loi. Chup man hinh cua so nay gui cho Claude nhe.
  pause
  exit /b
)

echo.
echo [2/2] Dang nhap tung CLI (lam 1 lan duy nhat)
echo.
echo --- Claude: trinh duyet se mo de dang nhap tai khoan Claude.
echo     Dang nhap xong, go /exit roi Enter de sang buoc tiep.
pause
call claude

echo.
echo --- Codex: trinh duyet se mo de dang nhap tai khoan ChatGPT.
pause
call codex login

echo.
echo --- Gemini: chon "Login with Google", dang nhap xong go /quit roi Enter.
pause
call gemini

echo.
echo ==================================================
echo   XONG! Bam dup run.bat de mo Room Control AI.
echo ==================================================
pause
