@echo off
setlocal

REM Simple launcher that ensures .venv exists and runs the app with it.
set "VENV_PY=.venv\Scripts\python.exe"
set "REQ_FILE=requirements.txt"

REM Find a base Python to create the venv if needed.
set "BASE_PY="
for %%P in (python py) do (
    where %%P >nul 2>nul && (
        set "BASE_PY=%%P"
        goto :have_python
    )
)
echo Python was not found on PATH. Install Python 3.11+ and retry.
exit /b 1

:have_python
if exist "%VENV_PY%" goto :run_app

echo Creating virtual environment in .venv...
%BASE_PY% -m venv .venv || exit /b 1
echo Installing dependencies...
call "%VENV_PY%" -m pip install --upgrade pip
if exist "%REQ_FILE%" (
    call "%VENV_PY%" -m pip install -r "%REQ_FILE%" || exit /b 1
) else (
    echo %REQ_FILE% not found; skipping dependency install.
)

:run_app
call "%VENV_PY%" -m flow_stt %*
endlocal
