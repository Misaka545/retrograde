const { app, BrowserWindow, ipcMain, session } = require('electron');
const path = require('path');

// Khai báo biến win ở ngoài để không bị dọn dẹp bộ nhớ (Garbage Collection)
let win;

function createWindow() {
  // 1. Tạo cửa sổ
  win = new BrowserWindow({
    width: 1200,
    height: 800,
    frame: false, // Tắt khung mặc định của Windows để dùng Custom TitleBar
    backgroundColor: '#09090b',
    icon: path.join(__dirname, '../build/icon.ico'),
    webPreferences: {
      nodeIntegration: true,
      contextIsolation: false,
      webSecurity: false,      // Cho phép load file local (nhạc)
    },
  });

  // 2. Cấu hình quyền truy cập thiết bị 
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

  // 3. Load ứng dụng (React)
  // Nếu đang chạy dev (npm run dev), load localhost. Nếu build xong, load file index.html
  const startUrl = process.env.ELECTRON_START_URL || `file://${path.join(__dirname, '../dist/index.html')}`;
  win.loadURL(startUrl);

  // Mở DevTools khi chạy Dev (Tuỳ chọn)
  // win.webContents.openDevTools();

  // Xử lý sự kiện đóng cửa sổ
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
  if (win) win.close();
});

app.on('ready', createWindow);

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit();
  }
});

app.on('activate', () => {
  if (win === null) {
    createWindow();
  }
});