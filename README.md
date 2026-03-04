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
| ![Library View](<img width="1495" height="997" alt="image" src="https://github.com/user-attachments/assets/8efe9b71-58e1-49a5-87ab-b1654abc27fb" />
) | ![Player View](<img width="1491" height="993" alt="image" src="https://github.com/user-attachments/assets/0fb6562d-127b-4ab4-935a-a630f57c3ea1" />
) |

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
