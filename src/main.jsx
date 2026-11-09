import React from 'react'
import ReactDOM from 'react-dom/client'
import { Buffer } from 'buffer';
window.Buffer = Buffer;

import App from './App.jsx'
import './index.css'

const syncIdleClass = () => {
  const idle = document.hidden || !document.hasFocus();
  document.documentElement.classList.toggle('app-idle', idle);
};
window.addEventListener('focus', syncIdleClass);
window.addEventListener('blur', syncIdleClass);
document.addEventListener('visibilitychange', syncIdleClass);
syncIdleClass();

ReactDOM.createRoot(document.getElementById('root')).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)