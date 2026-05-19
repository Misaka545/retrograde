// src/utils/db.js

let fs, path, os, nativeImage;

if (typeof window !== 'undefined' && window.require) {
    fs = window.require('fs');
    path = window.require('path');
    os = window.require('os');
    try {
        nativeImage = window.require('electron').nativeImage;
    } catch(e) {}
}

const getLibraryPath = () => {
    if (!os || !path || !fs) return null;
    const homeDir = os.homedir();
    const appDir = path.join(homeDir, '.retrograde');
    if (!fs.existsSync(appDir)) {
        fs.mkdirSync(appDir, { recursive: true });
    }
    return path.join(appDir, 'library.json');
};

const readLibrarySync = () => {
    const libPath = getLibraryPath();
    if (!libPath || !fs.existsSync(libPath)) {
        return {};
    }
    try {
        const data = fs.readFileSync(libPath, 'utf8');
        return JSON.parse(data);
    } catch (err) {
        console.error("Failed to read library.json", err);
        return {};
    }
};

const writeLibrarySync = (library) => {
    const libPath = getLibraryPath();
    if (!libPath) return;
    try {
        fs.writeFileSync(libPath, JSON.stringify(library), 'utf8');
    } catch (err) {
        console.error("Failed to write library.json", err);
    }
};


export const saveAlbumToDB = async (album) => {
    const library = readLibrarySync();
    library[album.name] = album;
    writeLibrarySync(library);
};


export const getAllAlbumsFromDB = async () => {
    const library = readLibrarySync();
    return Object.values(library);
};


const hashName = (str) => {
    let hash = 0;
    for (let i = 0; i < str.length; i++) {
        const chr = str.charCodeAt(i);
        hash = ((hash << 5) - hash) + chr;
        hash |= 0;
    }
    return Math.abs(hash).toString(36);
};

const deleteCoverFiles = (albumName) => {
    if (!os || !path || !fs) return;
    const coverDir = path.join(os.homedir(), '.retrograde', 'covers');
    const safeId = hashName(albumName);
    const thumbPath = path.join(coverDir, `${safeId}_thumb.jpg`);
    const fullPath = path.join(coverDir, `${safeId}_full.jpg`);
    try { if (fs.existsSync(thumbPath)) fs.unlinkSync(thumbPath); } catch(e) {}
    try { if (fs.existsSync(fullPath)) fs.unlinkSync(fullPath); } catch(e) {}
};

export const saveCoverArt = (albumName, pictureData, mimeType) => {
    if (!os || !path || !fs) return null;
    const homeDir = os.homedir();
    const coverDir = path.join(homeDir, '.retrograde', 'covers');
    if (!fs.existsSync(coverDir)) {
        fs.mkdirSync(coverDir, { recursive: true });
    }
    
    const safeId = hashName(albumName);
    const thumbPath = path.join(coverDir, `${safeId}_thumb.jpg`);
    const fullPath = path.join(coverDir, `${safeId}_full.jpg`);
    
    const rawBuffer = Buffer.from(pictureData);

    // Save full-resolution original
    if (!fs.existsSync(fullPath)) {
        try {
            if (nativeImage) {
                const img = nativeImage.createFromBuffer(rawBuffer);
                fs.writeFileSync(fullPath, img.toJPEG(95));
            } else {
                fs.writeFileSync(fullPath, rawBuffer);
            }
        } catch (e) {
            console.error("Failed to save full cover art", e);
        }
    }

    // Save thumbnail (resize by width only to preserve aspect ratio)
    if (!fs.existsSync(thumbPath)) {
        try {
            if (nativeImage) {
                const img = nativeImage.createFromBuffer(rawBuffer);
                const size = img.getSize();
                if (size.width > 300) {
                    const resized = img.resize({ width: 300, quality: 'good' });
                    fs.writeFileSync(thumbPath, resized.toJPEG(80));
                } else {
                    fs.writeFileSync(thumbPath, img.toJPEG(80));
                }
            } else {
                fs.writeFileSync(thumbPath, rawBuffer);
            }
        } catch (e) {
            console.error("Failed to save thumb cover art", e);
        }
    }
    
    const thumbUrl = `file://${thumbPath.replace(/\\/g, '/')}`;
    const fullUrl = `file://${fullPath.replace(/\\/g, '/')}`;
    return { thumb: thumbUrl, full: fullUrl };
};

export const saveLibraryToDB = async (library) => {
    const cleanLibrary = {};
    for (const albumName in library) {
        const album = library[albumName];
        cleanLibrary[albumName] = {
            name: album.name, artist: album.artist,
            coverArt: album.coverArt,
            coverArtFull: album.coverArtFull,
            tracks: album.tracks.map(t => ({
                id: t.id, title: t.title, artist: t.artist, album: t.album,
                duration: t.duration, filePath: t.filePath, src: t.src
            }))
        };
    }
    writeLibrarySync(cleanLibrary);
};

export const deleteAlbumFromDB = async (albumName) => {
    const library = readLibrarySync();
    if (library[albumName]) {
        delete library[albumName];
        writeLibrarySync(library);
        deleteCoverFiles(albumName);
    }
};

export const batchDeleteAlbumsFromDB = async (albumNames) => {
    const library = readLibrarySync();
    let changed = false;
    for (const name of albumNames) {
        if (library[name]) {
            delete library[name];
            changed = true;
            deleteCoverFiles(name);
        }
    }
    if (changed) writeLibrarySync(library);
};