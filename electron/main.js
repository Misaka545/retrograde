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
    if (!app.isQuiting) {
      event.preventDefault();
      
      const config = getConfig();
      
      if (config.minimizeToTray === null) {
        const choice = dialog.showMessageBoxSync(win, {
          type: 'question',
          buttons: ['Minimize to Tray', 'Quietly Exit'],
          defaultId: 0,
          cancelId: 1,
          title: 'Retrograde Background Play',
          message: 'Keep music playing in the background?',
          detail: 'If you choose Minimize, the player will stay active in your system tray when closed. You can toggle this setting anytime from the tray icon right-click menu.',
          checkboxLabel: 'Remember my choice',
          checkboxChecked: true
        });

        const shouldMinimize = choice.response === 0;
        
        if (choice.checkboxChecked) {
          config.minimizeToTray = shouldMinimize;
          saveConfig(config);
          updateTrayMenu();
        }
        
        if (shouldMinimize) {
          win.hide();
        } else {
          app.isQuiting = true;
          app.quit();
        }
      } 
      else if (config.minimizeToTray === true) {
        win.hide();
      } else {
        app.isQuiting = true;
        app.quit();
      }
      return false;
    }
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
  if (win) {
    win.close();
  }
});

function createTray() {
  const iconPath = path.join(__dirname, '../build/icon.ico');
  tray = new Tray(iconPath);
  
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
}

app.on('ready', () => {
  createWindow();
  createTray();
});

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