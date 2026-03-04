# Retrograde // Music Player

![Version](https://img.shields.io/badge/VERSION-1.0-4FD6BE?style=for-the-badge&logo=appveyor)
![System Status](https://img.shields.io/badge/SYSTEM-ONLINE-FF6B35?style=for-the-badge)
![Platform](https://img.shields.io/badge/PLATFORM-ELECTRON%20%2F%20WEB-E8C060?style=for-the-badge)

> *"If, in a century or a millennium, our descendants walk among the stars, the masses will sing her praises."*

**Retrograde** is a high-fidelity, local music player built with a futuristic, terminal-inspired interface. Designed for audiophiles who appreciate visual immersion, Retrograde focuses on offline data persistence, a distinct cyberpunk aesthetic, and unique interactive elements hidden within the UI.

---

## Interface Preview

| **Library Grid View** | **Full Screen Player** |
|:---:|:---:|
| ![Library View](https://scontent.fsgn2-9.fna.fbcdn.net/v/t1.15752-9/591390006_698148316422472_7689848877718754895_n.png?stp=dst-png_s552x414&_nc_cat=106&ccb=1-7&_nc_sid=0024fc&_nc_eui2=AeFTkUdF2md_H93nx3jPf_rLlzVmPRaK7MKXNWY9ForswuMJmQbiteSMs04KiJypaUUF8b_O4crKOcwYSqFPnLYz&_nc_ohc=jGH32VPEwbwQ7kNvwGysdpO&_nc_oc=AdlFKSjThbL7W_y95MM81RI1V2F3gwjngO1d174ReAXtTTr98MsaX310miU-PvlCtVU&_nc_ad=z-m&_nc_cid=0&_nc_zt=23&_nc_ht=scontent.fsgn2-9.fna&oh=03_Q7cD4AHV62rGLkfLn5zh-t3hTYwz3QE1-1Rzu-ZIbQixZai9Lw&oe=695B97F0) | ![Player View](https://scontent.fsgn2-9.fna.fbcdn.net/v/t1.15752-9/590276689_2993010477567014_26912626367296599_n.png?stp=dst-png_s552x414&_nc_cat=103&ccb=1-7&_nc_sid=0024fc&_nc_eui2=AeFVXYzpPErKbNauSdB2YQ9EiUMr7aP8JimJQyvto_wmKfiqEyHUv86QihGuAYh0RmDRhO-FYTjIAFU9vruAyMfI&_nc_ohc=IMcOi-SmwcsQ7kNvwGsolIK&_nc_oc=Adm8QaMr2Pe5f80s1A8RUMWby-xEVhtwBOuIWDEHqAe0zYUQzE20GtVmrzFheFVCxg0&_nc_ad=z-m&_nc_cid=0&_nc_zt=23&_nc_ht=scontent.fsgn2-9.fna&oh=03_Q7cD4AHoYlmDjUf9KA-r34o7LKS6c5k2s50q_w_-dOWMXu6F1w&oe=695BAEBB) |

---

## System Features

### Core Audio Protocol
* **Local Music Management:** Seamlessly scan folders or import individual files. Supports high-fidelity formats (`FLAC`, `WAV`, `MP3`).
* **Intelligent Metadata:** Automatic extraction of Title, Artist, and Cover Art via `music-metadata-browser`.
* **Persistent Memory (IndexedDB):** Stores your entire library and heavy cover art blobs directly in the browser/app database. Data persists even after the system shuts down.
* **Audio Engine:** Full HTML5 Audio API control (Play, Pause, Seek, Shuffle, Repeat, Volume).

### UI / UX Experience
* **Retro Futurism Aesthetic:** A high-contrast theme featuring **Teal** (`#4FD6BE`), **Gold** (`#E8C060`), and **Orange** (`#FF6B35`) against a void-black background.
* **Kinetic Visuals:** Interface includes spinning discs, pulsing tech indicators, and audio-reactive elements.
* **Focus Modes:**
    * **Sidebar Nav:** Integrated navigation with a hidden input visualizer.
    * **Full Screen:** Distraction-free listening with large album art and transparent queue overlay.

### Organization Module
* **Playlists:** Create and compile custom playlists.
* **Favorites:** Fast-track access to "Liked" tracks.
* **Search Engine:** Real-time filtering by Track Name, Artist, Album, or Playlist.

---

## Tech Stack

* **Runtime:** ![NodeJS](https://img.shields.io/badge/Node.js-339933?style=flat&logo=nodedotjs&logoColor=white) ![Electron](https://img.shields.io/badge/Electron-47848F?style=flat&logo=electron&logoColor=white)
* **Framework:** ![React](https://img.shields.io/badge/React-20232A?style=flat&logo=react&logoColor=61DAFB) ![Vite](https://img.shields.io/badge/Vite-646CFF?style=flat&logo=vite&logoColor=white)
* **Styling:** ![TailwindCSS](https://img.shields.io/badge/Tailwind_CSS-38B2AC?style=flat&logo=tailwind-css&logoColor=white)
* **Icons:** Lucide React
* **Data Handling:** Native IndexedDB API & `music-metadata-browser`

---

## Installation & Setup

### Prerequisites
* Node.js (v16+)
* npm or yarn

### Initialization Sequence

1.  **Clone the Repository**
    ```bash
    git clone https://github.com/Misaka545/retrograde.git
    cd music-player
    ```

2.  **Install Dependencies**
    ```bash
    npm install
    ```

3.  **Run Development Protocols**

    * *For Web Browser:*
        ```bash
        npm run dev
        ```

    * *For Desktop App (Electron):*
        ```bash
        npm run electron:dev
        ```

---

## 📜 License

Distributed under the MIT License. See `LICENSE` for more information.
