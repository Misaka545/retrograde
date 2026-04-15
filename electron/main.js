const { app, BrowserWindow, ipcMain, session, Tray, Menu, dialog } = require('electron');
const path = require('path');
const fs = require('fs');

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
const { nativeImage } = require('electron');
const os = require('os');

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