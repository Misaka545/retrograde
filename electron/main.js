const { app, BrowserWindow, ipcMain, session, Tray, Menu, dialog } = require('electron');
const path = require('path');
const fs = require('fs');
const mpvController = require('./mpv-controller');

let win;
let tray = null;
app.isQuiting = false;

const configPath = path.join(app.getPath('userData'), 'retrograde-config.json');

function getConfig() {
  try {
    if (fs.existsSync(configPath)) {
      return JSON.parse(fs.readFileSync(configPath, 'utf8'));
    }
  } catch(e) {}
  return { minimizeToTray: null };
}

function saveConfig(config) {
  try {
    fs.writeFileSync(configPath, JSON.stringify(config, null, 2));
  } catch(e) {}
}

function updateTrayMenu() {
  if (!tray) return;
  const config = getConfig();
  
  const contextMenu = Menu.buildFromTemplate([
    { label: 'Show Player', click: () => { if (win) { win.show(); } else { createWindow(); } } },
    { type: 'separator' },
    { 
      label: 'Minimize to tray on close', 
      type: 'checkbox', 
      checked: config.minimizeToTray !== false, 
      click: (menuItem) => {
        const currentConfig = getConfig();
        currentConfig.minimizeToTray = menuItem.checked;
        saveConfig(currentConfig);
      }
    },
    { type: 'separator' },
    { label: 'Exit', click: () => { app.isQuiting = true; app.quit(); } }
  ]);
  
  tray.setContextMenu(contextMenu);
}

function handleWindowClose(event) {
  if (app.isQuiting) return;

  if (event) {
    event.preventDefault();
  }
  
  const config = getConfig();
  
  if (config.minimizeToTray === null || config.minimizeToTray === undefined) {
    if (win) {
      win.webContents.send('request-tray-minimize-info');
    }
  } 
  else if (config.minimizeToTray === true) {
    if (win) win.hide();
  } else {
    app.isQuiting = true;
    mpvController.quit();
    app.quit();
  }
}

ipcMain.on('tray-minimize-response', (event, { shouldMinimize, rememberChoice }) => {
  const config = getConfig();
  
  if (rememberChoice) {
    config.minimizeToTray = shouldMinimize;
    saveConfig(config);
    updateTrayMenu();
  }
  
  if (shouldMinimize) {
    if (win) win.hide();
  } else {
    app.isQuiting = true;
    mpvController.quit();
    app.quit();
  }
});

function createWindow() {
  win = new BrowserWindow({
    width: 1200,
    height: 800,
    frame: false, 
    backgroundColor: '#09090b',
    icon: path.join(__dirname, '../build/icon.ico'),
    webPreferences: {
      nodeIntegration: true,
      contextIsolation: false,
      webSecurity: false,      
    },
  });

  if (win.webContents.session) {
    win.webContents.session.setPermissionCheckHandler((webContents, permission, requestingOrigin) => {
      if (permission === 'media' || permission === 'audioCapture') {
        return true;
      }
      return false;
    });

    win.webContents.session.setPermissionRequestHandler((webContents, permission, callback) => {
      if (permission === 'media' || permission === 'audioCapture') {
        return callback(true);
      }
      return callback(false);
    });
  }

  const startUrl = process.env.ELECTRON_START_URL || `file://${path.join(__dirname, '../dist/index.html')}`;
  win.loadURL(startUrl);

  win.on('close', (event) => {
    handleWindowClose(event);
  });

  win.on('closed', () => {
    win = null;
  });

  win.webContents.on('console-message', (event, level, message, line, sourceId) => {
    console.log(`[Renderer] ${message} (${sourceId}:${line})`);
  });
}

ipcMain.on('window-minimize', () => {
  if (win) win.minimize();
});

ipcMain.on('window-maximize', () => {
  if (win) {
    if (win.isMaximized()) {
      win.unmaximize();
    } else {
      win.maximize();
    }
  }
});


ipcMain.on('window-close', () => {
  handleWindowClose(null);
});

// mpv reports time-pos many times per second; the UI only shows whole seconds.
// Forward at most ~4Hz while visible (1Hz when hidden). Seeks/jumps go through at once,
// and a trailing send delivers the latest value, since mpv's pipe can deliver updates in bursts.
let lastTimePosSent = -1;
let lastTimePosAt = 0;
let latestTimePos = null;
let timePosTimer = null;
const sendTimePos = () => {
  timePosTimer = null;
  if (!win || latestTimePos === null) return;
  lastTimePosSent = latestTimePos;
  lastTimePosAt = Date.now();
  latestTimePos = null;
  win.webContents.send('mpv-time-pos', lastTimePosSent);
};
mpvController.on('time-pos', (time) => {
  if (!win) return;
  latestTimePos = time;
  const minInterval = win.isVisible() ? 250 : 1000;
  const jumped = lastTimePosSent < 0 || time < lastTimePosSent || time - lastTimePosSent > 1.5;
  const wait = minInterval - (Date.now() - lastTimePosAt);
  if (jumped || wait <= 0) {
    if (timePosTimer) clearTimeout(timePosTimer);
    sendTimePos();
  } else if (!timePosTimer) {
    timePosTimer = setTimeout(sendTimePos, wait);
  }
});
mpvController.on('pause', (isPaused) => {
  if (win) win.webContents.send('mpv-pause', isPaused);
});
mpvController.on('eof-reached', () => {
  if (win) win.webContents.send('mpv-eof');
});
mpvController.on('file-loaded', () => {
  if (win) win.webContents.send('mpv-file-loaded');
});

ipcMain.handle('mpv-start', async () => {
  try {
    await mpvController.start();
    return { success: true };
  } catch (err) {
    return { success: false, error: err.message };
  }
});

ipcMain.handle('mpv-play', async (event, filePath, startTime = 0) => {
  try {
    const src = filePath.startsWith('file://') ? filePath : `file://${filePath}`;
    await mpvController.play(src, startTime);
    return { success: true };
  } catch (err) {
    return { success: false, error: err.message };
  }
});

ipcMain.handle('mpv-pause', async () => {
  try {
    await mpvController.pause();
    return { success: true };
  } catch (err) {
    return { success: false, error: err.message };
  }
});

ipcMain.handle('mpv-resume', async () => {
  try {
    await mpvController.resume();
    return { success: true };
  } catch (err) {
    return { success: false, error: err.message };
  }
});

ipcMain.handle('mpv-seek', async (event, seconds) => {
  try {
    await mpvController.seek(seconds);
    return { success: true };
  } catch (err) {
    return { success: false, error: err.message };
  }
});

ipcMain.handle('mpv-set-volume', async (event, volume) => {
  try {
    await mpvController.setVolume(volume);
    return { success: true };
  } catch (err) {
    return { success: false, error: err.message };
  }
});

ipcMain.handle('mpv-quit', async () => {
  mpvController.quit();
  return { success: true };
});

ipcMain.handle('mpv-stop', async () => {
  mpvController.quit();
  return { success: true };
});

ipcMain.handle('mpv-toggle-pause', async () => {
  try {
    await mpvController.togglePause();
    return { success: true };
  } catch (err) {
    return { success: false, error: err.message };
  }
});

const { nativeImage } = require('electron');
const os = require('os');
const { discoverAudioFiles, scanFiles, readLibrarySync } = require('./scanner');

ipcMain.handle('scan-folder', async () => {
  if (!win) return { cancelled: true };

  const result = await dialog.showOpenDialog(win, {
    properties: ['openDirectory'],
    title: 'Select Music Folder'
  });

  if (result.canceled || result.filePaths.length === 0) {
    return { cancelled: true };
  }

  const folderPath = result.filePaths[0];
  win.webContents.send('scan-progress', { phase: 'discovering', scanned: 0, total: 0, currentFile: `Scanning ${folderPath}...` });
  const audioFiles = discoverAudioFiles(folderPath);

  if (audioFiles.length === 0) {
    return { cancelled: false, count: 0 };
  }

  const existingLibrary = readLibrarySync();
  const library = await scanFiles(audioFiles, win, existingLibrary);

  win.webContents.send('scan-complete', { library });
  return { cancelled: false, count: audioFiles.length };
});

ipcMain.handle('scan-files', async () => {
  if (!win) return { cancelled: true };

  const result = await dialog.showOpenDialog(win, {
    properties: ['openFile', 'multiSelections'],
    title: 'Select Music Files',
    filters: [
      { name: 'Audio Files', extensions: ['flac', 'mp3', 'wav', 'aac', 'ogg', 'm4a', 'wma', 'aiff', 'alac', 'opus'] }
    ]
  });

  if (result.canceled || result.filePaths.length === 0) {
    return { cancelled: true };
  }

  const existingLibrary = readLibrarySync();
  const library = await scanFiles(result.filePaths, win, existingLibrary);

  win.webContents.send('scan-complete', { library });
  return { cancelled: false, count: result.filePaths.length };
});

ipcMain.handle('scan-dropped-paths', async (event, paths) => {
  if (!win || !paths || paths.length === 0) return { cancelled: true };
  const fs = require('fs');
  const path = require('path');
  const AUDIO_EXTENSIONS = new Set(['.flac', '.mp3', '.wav', '.aac', '.ogg', '.m4a', '.wma', '.aiff', '.alac', '.opus']);
  
  let allAudioFiles = [];
  
  win.webContents.send('scan-progress', { phase: 'discovering', scanned: 0, total: 0, currentFile: `Scanning dropped items...` });
  
  for (const p of paths) {
    try {
      const stat = fs.statSync(p);
      if (stat.isDirectory()) {
        allAudioFiles.push(...discoverAudioFiles(p));
      } else if (stat.isFile()) {
        const ext = path.extname(p).toLowerCase();
        if (AUDIO_EXTENSIONS.has(ext)) {
          allAudioFiles.push(p);
        }
      }
    } catch (e) {
      console.error(`Failed to stat dropped path: ${p}`, e);
    }
  }
  
  if (allAudioFiles.length === 0) {
    return { cancelled: false, count: 0 };
  }
  
  const existingLibrary = readLibrarySync();
  const library = await scanFiles(allAudioFiles, win, existingLibrary);
  
  win.webContents.send('scan-complete', { library });
  return { cancelled: false, count: allAudioFiles.length };
});

ipcMain.handle('regenerate-thumbnails', async () => {
    const coverDir = path.join(os.homedir(), '.retrograde', 'covers');
    if (!fs.existsSync(coverDir)) return { regenerated: 0 };
    // One-time migration: skip the per-file stat pass on every later startup.
    const doneMarker = path.join(coverDir, '.thumbs-v400');
    if (fs.existsSync(doneMarker)) return { regenerated: 0 };

    const files = fs.readdirSync(coverDir).filter(f => f.endsWith('_thumb.jpg'));
    let count = 0;

    for (const thumbFile of files) {
        const thumbPath = path.join(coverDir, thumbFile);
        try {
            await new Promise(resolve => setImmediate(resolve));
            const stat = fs.statSync(thumbPath);
            if (stat.size > 100 * 1024) {
                const fullFile = thumbFile.replace('_thumb.jpg', '_full.jpg');
                const fullPath = path.join(coverDir, fullFile);

                if (fs.existsSync(fullPath)) {
                    const raw = fs.readFileSync(fullPath);
                    const img = nativeImage.createFromBuffer(raw);
                    const size = img.getSize();
                    if (size.width > 400 && size.width > 0) {
                        const resized = img.resize({
                            width: 400,
                            height: Math.round(size.height * (400 / size.width)),
                            quality: 'good'
                        });
                        fs.writeFileSync(thumbPath, resized.toJPEG(80));
                        count++;
                    }
                }
            }
        } catch (e) { /* skip */ }
    }

    console.log(`[Migration] Regenerated ${count} thumbnails`);
    try { fs.writeFileSync(doneMarker, ''); } catch (e) { /* retry next start */ }
    return { regenerated: count };
});

function getSafeTrayIconPath() {
  // 1. Production: electron-builder copies icon.ico to resources/ outside ASAR
  const resourceIcon = path.join(process.resourcesPath || '', 'icon.ico');
  if (fs.existsSync(resourceIcon)) return resourceIcon;

  // 2. Development: icon is in build/
  const devIcon = path.join(__dirname, '../build/icon.ico');
  if (fs.existsSync(devIcon)) return devIcon;

  // 3. Fallback: write a tiny png to %TEMP%
  const tmpPath = path.join(os.tmpdir(), 'retrograde_tray_icon.png');
  const base64Data = 'iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2AAAAAXNSR0IArs4c6QAAAARnQU1BAACxjwv8YQUAAAAJcEhZcwAAEnQAABJ0Ad5mH3gAAAAcSURBVDhPY/zPwAAEQGZMAmEUS2gYNY0YMBBQAwHhL10o4MfjAAAAAElFTkSuQmCC';
  try {
    fs.writeFileSync(tmpPath, Buffer.from(base64Data, 'base64'));
    return tmpPath;
  } catch(e) {
    return null;
  }
}

function createTray() {
  try {
    const iconSafePath = getSafeTrayIconPath();
    if (!iconSafePath) {
      console.error("No valid tray icon path found – tray skipped.");
      return;
    }
    tray = new Tray(iconSafePath);
    
    tray.setToolTip('Retrograde');
    updateTrayMenu();
    
    tray.on('click', () => {
      if (win) {
        if (win.isVisible()) {
          win.hide();
        } else {
          win.show();
        }
      } else {
        createWindow();
      }
    });
  } catch (err) {
    console.error("Tray creation failed:", err);
  }
}

const gotTheLock = app.requestSingleInstanceLock();

if (!gotTheLock) {
  app.quit();
} else {
  app.on('second-instance', (event, commandLine, workingDirectory) => {
    if (win) {
      if (win.isMinimized()) win.restore();
      if (!win.isVisible()) win.show();
      win.focus();
    } else {
      createWindow();
    }
  });

  app.on('ready', () => {
    createWindow();
    createTray();
  });
}

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin' && app.isQuiting) {
    app.quit();
  }
});

app.on('activate', () => {
  if (win === null) {
    createWindow();
  }
});