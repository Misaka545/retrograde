// electron/scanner-worker.js
const { parentPort } = require('worker_threads');
const path = require('path');
const { parseFlacFile } = require('./flac-parser');

let mm = null; 

//Parse a non-FLAC file using music-metadata (fallback).
async function parseWithMusicMetadata(filePath) {
    if (!mm) mm = require('music-metadata');
    try {
        const metadata = await mm.parseFile(filePath, {
            duration: true,
            skipCovers: true,
        });
        const common = metadata.common || {};
        const format = metadata.format || {};
        return {
            filePath,
            title: common.title || path.basename(filePath, path.extname(filePath)),
            artist: common.artist || 'Unknown Artist',
            album: common.album || 'Unknown Album',
            duration: format.duration || 0,
            trackNumber: common.track?.no || 0,
            year: common.year || 0,
            codec: format.codec || path.extname(filePath).replace('.', '').toUpperCase(),
            sampleRate: format.sampleRate || 0,
            bitsPerSample: format.bitsPerSample || 0,
            channels: format.numberOfChannels || 0,
            hasPicture: !!(common.picture && common.picture.length > 0),
            pictureOffset: null,
            pictureSize: null,
        };
    } catch (err) {
        return { filePath, error: err.message };
    }
}

parentPort.on('message', async (msg) => {
    if (msg.type === 'parse-batch') {
        const results = [];
        for (const filePath of msg.files) {
            const ext = path.extname(filePath).toLowerCase();
            let result;

            if (ext === '.flac') {
                result = parseFlacFile(filePath);
                if (!result) {
                    result = await parseWithMusicMetadata(filePath);
                }
            } else {
                result = await parseWithMusicMetadata(filePath);
            }

            results.push(result);
            parentPort.postMessage({ type: 'file-done', filePath });
        }
        parentPort.postMessage({ type: 'batch-done', results, batchId: msg.batchId });
    }
});
