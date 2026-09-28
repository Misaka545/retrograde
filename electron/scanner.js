// electron/scanner.js
const { Worker } = require('worker_threads');
const path = require('path');
const fs = require('fs');
const os = require('os');
const { extractFlacPicture } = require('./flac-parser');

const WORKER_COUNT = Math.min(4, os.cpus().length);
const AUDIO_EXTENSIONS = new Set(['.flac', '.mp3', '.wav', '.aac', '.ogg', '.m4a', '.wma', '.aiff', '.alac', '.opus']);

//Recursively find all audio files in a directory.
function discoverAudioFiles(dirPath) {
    const results = [];
    const stack = [dirPath];

    while (stack.length > 0) {
        const currentDir = stack.pop();
        let entries;
        try {
            entries = fs.readdirSync(currentDir, { withFileTypes: true });
        } catch (err) {
            continue;
        }

        for (const entry of entries) {
            const fullPath = path.join(currentDir, entry.name);
            if (entry.isDirectory()) {
                stack.push(fullPath);
            } else if (entry.isFile()) {
                const ext = path.extname(entry.name).toLowerCase();
                if (AUDIO_EXTENSIONS.has(ext)) {
                    results.push(fullPath);
                }
            }
        }
    }
    return results;
}

//Hash an album name for cover file naming (same logic as db.js).
function hashName(str) {
    let hash = 0;
    for (let i = 0; i < str.length; i++) {
        const chr = str.charCodeAt(i);
        hash = ((hash << 5) - hash) + chr;
        hash |= 0;
    }
    return Math.abs(hash).toString(36);
}

//Save cover art to disk. Accepts raw Buffer (not base64).
//Returns { thumb, full } URLs or null.
function saveCoverArtBuffer(albumName, rawBuffer, mimeType) {
    try {
        const { nativeImage } = require('electron');
        const homeDir = os.homedir();
        const coverDir = path.join(homeDir, '.retrograde', 'covers');
        if (!fs.existsSync(coverDir)) {
            fs.mkdirSync(coverDir, { recursive: true });
        }

        const safeId = hashName(albumName);
        const thumbPath = path.join(coverDir, `${safeId}_thumb.jpg`);
        const fullPath = path.join(coverDir, `${safeId}_full.jpg`);

        if (!fs.existsSync(fullPath)) {
            try {
                fs.writeFileSync(fullPath, rawBuffer);
            } catch (e) {
                console.error("Failed to save full cover art", e);
            }
        }

        if (!fs.existsSync(thumbPath)) {
            try {
                const img = nativeImage.createFromBuffer(rawBuffer);
                const size = img.getSize();
                if (size.width > 400 && size.width > 0) {
                    const resized = img.resize({
                        width: 400,
                        height: Math.round(size.height * (400 / size.width)),
                        quality: 'good'
                    });
                    fs.writeFileSync(thumbPath, resized.toJPEG(80));
                } else {
                    fs.writeFileSync(thumbPath, rawBuffer);
                }
            } catch (e) {
                console.error("Failed to resize thumb, saving raw:", e);
                fs.writeFileSync(thumbPath, rawBuffer);
            }
        }

        const thumbUrl = `file://${thumbPath.replace(/\\/g, '/')}`;
        const fullUrl = `file://${fullPath.replace(/\\/g, '/')}`;
        return { thumb: thumbUrl, full: fullUrl };
    } catch (err) {
        console.error('Failed to save cover art:', err);
        return null;
    }
}

/**
 * Main scan function — orchestrates workers to parse audio files.
 * @param {string[]} filePaths - Array of file paths to scan
 * @param {BrowserWindow} win - Electron BrowserWindow for IPC events
 * @param {object} existingLibrary - Current library to merge into
 * @returns {Promise<object>} - The merged library object
 */
async function scanFiles(filePaths, win, existingLibrary = {}) {
    return new Promise((resolve, reject) => {
        const totalFiles = filePaths.length;
        let scannedCount = 0;
        const allResults = [];
        let completedWorkers = 0;
        const startTime = Date.now();

        if (totalFiles === 0) {
            resolve(existingLibrary);
            return;
        }

        win.webContents.send('scan-progress', {
            phase: 'scanning',
            scanned: 0,
            total: totalFiles,
            currentFile: '',
        });

        const workerFileLists = Array.from({ length: WORKER_COUNT }, () => []);
        filePaths.forEach((fp, i) => {
            workerFileLists[i % WORKER_COUNT].push(fp);
        });

        const activeWorkerFiles = workerFileLists.filter(list => list.length > 0);
        const activeWorkerCount = activeWorkerFiles.length;
        const workers = [];

        for (let w = 0; w < activeWorkerCount; w++) {
            const worker = new Worker(path.join(__dirname, 'scanner-worker.js'));
            workers.push(worker);

            worker.on('message', (msg) => {
                if (msg.type === 'file-done') {
                    scannedCount++;
                    const interval = Math.max(1, Math.floor(totalFiles / 100));
                    if (scannedCount % interval === 0 || scannedCount === totalFiles) {
                        win.webContents.send('scan-progress', {
                            phase: 'scanning',
                            scanned: scannedCount,
                            total: totalFiles,
                            currentFile: path.basename(msg.filePath),
                        });
                    }
                }

                if (msg.type === 'batch-done') {
                    allResults.push(...msg.results);
                    completedWorkers++;

                    if (completedWorkers === activeWorkerCount) {
                        const scanMs = Date.now() - startTime;
                        console.log(`[Scanner] Metadata scan: ${scanMs}ms for ${totalFiles} files`);

                        for (const w of workers) w.terminate();

                        processResults(allResults, win, existingLibrary)
                            .then(resolve)
                            .catch(reject);
                    }
                }
            });

            worker.on('error', (err) => {
                console.error('Worker error:', err);
            });

            worker.postMessage({
                type: 'parse-batch',
                files: activeWorkerFiles[w],
                batchId: w,
            });
        }
    });
}

//Process scan results: group into albums, extract covers directly, build library.
async function processResults(results, win, existingLibrary) {
    const startTime = Date.now();
    const tempLibrary = { ...existingLibrary };

    // albumCoverSource: albumName -> { filePath, pictureOffset, pictureSize }
    const albumCoverSource = {};

    for (const result of results) {
        if (result.error) continue;

        const albumName = result.album;
        const artist = result.artist;

        if (!tempLibrary[albumName]) {
            tempLibrary[albumName] = {
                name: albumName,
                artist: artist,
                coverArt: null,
                coverArtFull: null,
                year: 0,
                addedAt: Date.now(),
                tracks: [],
            };
        }

        // Update album year from first track that has one
        if (!tempLibrary[albumName].year && result.year) {
            tempLibrary[albumName].year = result.year;
        }

        if (!albumCoverSource[albumName] && result.hasPicture) {
            albumCoverSource[albumName] = {
                filePath: result.filePath,
                pictureOffset: result.pictureOffset,
                pictureSize: result.pictureSize,
            };
        }

        const fileSrc = `file://${result.filePath.replace(/\\/g, '/')}`;
        const alreadyExists = tempLibrary[albumName].tracks.some(t => t.filePath === result.filePath);

        if (!alreadyExists) {
            tempLibrary[albumName].tracks.push({
                id: result.filePath,
                title: result.title,
                artist: result.artist,
                album: albumName,
                duration: result.duration,
                filePath: result.filePath,
                src: fileSrc,
                trackNumber: result.trackNumber,
                codec: result.codec || '',
                sampleRate: result.sampleRate || 0,
                bitsPerSample: result.bitsPerSample || 0,
                channels: result.channels || 0,
            });
        }
    }

    for (const albumName of Object.keys(tempLibrary)) {
        tempLibrary[albumName].tracks.sort((a, b) => (a.trackNumber || 0) - (b.trackNumber || 0));
    }

    const albumsNeedingCover = Object.keys(albumCoverSource).filter(
        name => !tempLibrary[name].coverArt
    );

    if (albumsNeedingCover.length > 0) {
        win.webContents.send('scan-progress', {
            phase: 'covers',
            scanned: 0,
            total: albumsNeedingCover.length,
            currentFile: 'Extracting covers...',
        });

        for (let i = 0; i < albumsNeedingCover.length; i++) {
            const albumName = albumsNeedingCover[i];
            const source = albumCoverSource[albumName];

            await new Promise(resolve => setImmediate(resolve));

            let picResult = null;

            if (source.pictureOffset !== null && source.pictureSize !== null) {
                // FLAC: Direct random-access read of picture block
                picResult = extractFlacPicture(source.filePath, source.pictureOffset, source.pictureSize);
            } else {
                // Non-FLAC: fall back to music-metadata for cover extraction
                try {
                    const mm = require('music-metadata');
                    const metadata = await mm.parseFile(source.filePath, { duration: false, skipCovers: false });
                    if (metadata.common.picture && metadata.common.picture.length > 0) {
                        const pic = metadata.common.picture[0];
                        picResult = { data: Buffer.from(pic.data), format: pic.format };
                    }
                } catch (e) {
                    console.error('Cover extraction failed:', e.message);
                }
            }

            if (picResult && picResult.data) {
                const saved = saveCoverArtBuffer(albumName, picResult.data, picResult.format);
                if (saved) {
                    tempLibrary[albumName].coverArt = saved.thumb;
                    tempLibrary[albumName].coverArtFull = saved.full;
                    for (const track of tempLibrary[albumName].tracks) {
                        track.coverArt = saved.thumb;
                        track.coverArtFull = saved.full;
                    }
                }
            }

            win.webContents.send('scan-progress', {
                phase: 'covers',
                scanned: i + 1,
                total: albumsNeedingCover.length,
                currentFile: albumName,
            });
        }
    }

    const totalMs = Date.now() - startTime;
    console.log(`[Scanner] Process + covers: ${totalMs}ms for ${albumsNeedingCover.length} albums`);

    saveLibrarySync(tempLibrary);

    return tempLibrary;
}

//Save library to disk.
function saveLibrarySync(library) {
    const homeDir = os.homedir();
    const appDir = path.join(homeDir, '.retrograde');
    if (!fs.existsSync(appDir)) {
        fs.mkdirSync(appDir, { recursive: true });
    }
    const libPath = path.join(appDir, 'library.json');

    const cleanLibrary = {};
    for (const albumName in library) {
        const album = library[albumName];
        cleanLibrary[albumName] = {
            name: album.name,
            artist: album.artist,
            coverArt: album.coverArt,
            coverArtFull: album.coverArtFull,
            year: album.year || 0,
            addedAt: album.addedAt || 0,
            tracks: album.tracks.map(t => ({
                id: t.id,
                title: t.title,
                artist: t.artist,
                album: t.album,
                duration: t.duration,
                filePath: t.filePath,
                src: t.src,
                codec: t.codec || '',
                sampleRate: t.sampleRate || 0,
                bitsPerSample: t.bitsPerSample || 0,
                channels: t.channels || 0,
            })),
        };
    }

    try {
        fs.writeFileSync(libPath, JSON.stringify(cleanLibrary), 'utf8');
    } catch (err) {
        console.error('Failed to write library.json:', err);
    }
}

//Read existing library from disk.
function readLibrarySync() {
    const homeDir = os.homedir();
    const libPath = path.join(homeDir, '.retrograde', 'library.json');
    if (!fs.existsSync(libPath)) return {};
    try {
        const data = fs.readFileSync(libPath, 'utf8');
        return JSON.parse(data);
    } catch (err) {
        return {};
    }
}

module.exports = {
    discoverAudioFiles,
    scanFiles,
    readLibrarySync,
};
