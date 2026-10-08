// electron/flac-parser.js

const fs = require('fs');
const path = require('path');

/**
 * Parse FLAC metadata by reading only the header bytes.
 * Total I/O per file: typically < 5KB (vs 30-50MB for full file read).
 *
 * FLAC structure:
 *   "fLaC" (4 bytes)
 *   METADATA_BLOCK* :
 *     Header: 4 bytes [is_last(1bit) | type(7bits) | size(24bits)]
 *     Data: <size> bytes
 *   AUDIO FRAMES (never read)
 *
 * Block types: 0=STREAMINFO, 1=PADDING, 2=APPLICATION, 3=SEEKTABLE,
 *              4=VORBIS_COMMENT, 5=CUESHEET, 6=PICTURE
 */
function parseFlacFile(filePath) {
    const fd = fs.openSync(filePath, 'r');
    try {
        let offset = 0;

        let header = Buffer.alloc(10);
        fs.readSync(fd, header, 0, 10, offset);

        if (header.toString('ascii', 0, 3) === 'ID3') {
            const id3Size = (header[6] << 21) | (header[7] << 14) | (header[8] << 7) | header[9];
            offset += 10 + id3Size;
        }

        const magic = Buffer.alloc(4);
        fs.readSync(fd, magic, 0, 4, offset);
        if (magic.toString('ascii') !== 'fLaC') {
            return null;
        }

        offset += 4;
        let streamInfo = null;
        let vorbisComment = null;
        let pictureOffset = null;
        let pictureSize = null;
        let isLast = false;

        const headerBuf = Buffer.alloc(4);

        while (!isLast) {
            fs.readSync(fd, headerBuf, 0, 4, offset);
            isLast = (headerBuf[0] & 0x80) !== 0;
            const blockType = headerBuf[0] & 0x7F;
            const blockSize = (headerBuf[1] << 16) | (headerBuf[2] << 8) | headerBuf[3];
            offset += 4;

            if (blockType === 0 && blockSize >= 34) {
                // STREAMINFO — always 34 bytes, contains sample rate & total samples
                const data = Buffer.alloc(34);
                fs.readSync(fd, data, 0, 34, offset);
                streamInfo = parseStreamInfo(data);
            } else if (blockType === 4) {
                // VORBIS_COMMENT — variable size, contains tags
                const data = Buffer.alloc(blockSize);
                fs.readSync(fd, data, 0, blockSize, offset);
                vorbisComment = parseVorbisComment(data);
            } else if (blockType === 6) {
                // PICTURE — just record offset, DO NOT read data
                pictureOffset = offset;
                pictureSize = blockSize;
            }
            // Skip to next block — NO read for PADDING/SEEKTABLE/PICTURE/etc
            offset += blockSize;
        }

        const tags = vorbisComment || {};
        const duration = streamInfo ? (streamInfo.totalSamples / streamInfo.sampleRate) : 0;
        const basename = path.basename(filePath, path.extname(filePath));

        return {
            filePath,
            title: tags.TITLE || basename,
            artist: tags.ARTIST || tags.ALBUMARTIST || 'Unknown Artist',
            album: tags.ALBUM || 'Unknown Album',
            trackNumber: parseInt(tags.TRACKNUMBER || '0', 10) || 0,
            discNumber: parseInt(tags.DISCNUMBER || '1', 10) || 1,
            year: parseInt(tags.DATE || tags.YEAR || '0', 10) || 0,
            duration,
            codec: 'FLAC',
            sampleRate: streamInfo?.sampleRate || 0,
            bitsPerSample: streamInfo?.bitsPerSample || 0,
            channels: streamInfo?.channels || 0,
            hasPicture: pictureOffset !== null,
            pictureOffset,
            pictureSize,
        };
    } catch (err) {
        return { filePath, error: err.message };
    } finally {
        fs.closeSync(fd);
    }
}

/**
 * Parse STREAMINFO block (34 bytes).
 * Layout:
 *   bytes 0-1:   min block size
 *   bytes 2-3:   max block size
 *   bytes 4-6:   min frame size
 *   bytes 7-9:   max frame size
 *   bytes 10-12: sample rate (20 bits)
 *   byte 12:     channels-1 (3 bits), bps-1 (5 bits, spans into byte 13)
 *   bytes 13-17: total samples (36 bits)
 */
function parseStreamInfo(data) {
    const sampleRate = (data[10] << 12) | (data[11] << 4) | (data[12] >>> 4);
    // channels-1 is 3 bits starting at bit 4 of byte 12
    const channels = ((data[12] & 0x0E) >>> 1) + 1;
    // bps-1 is 5 bits: 1 bit from byte 12 + 4 bits from byte 13
    const bitsPerSample = (((data[12] & 0x01) << 4) | ((data[13] & 0xF0) >>> 4)) + 1;
    // Total samples is 36 bits: 4 bits from byte 13 + bytes 14-17
    const totalSamples =
        (data[13] & 0x0F) * 0x100000000 +
        data[14] * 0x1000000 +
        data[15] * 0x10000 +
        data[16] * 0x100 +
        data[17];
    return { sampleRate, channels, bitsPerSample, totalSamples };
}

/**
 * Parse VORBIS_COMMENT block.
 * Layout:
 *   4 bytes LE: vendor string length
 *   N bytes:    vendor string (UTF-8)
 *   4 bytes LE: comment count
 *   For each comment:
 *     4 bytes LE: comment length
 *     N bytes:    comment string "KEY=VALUE" (UTF-8)
 */
function parseVorbisComment(data) {
    const tags = {};
    let offset = 0;

    if (data.length < 8) return tags;

    const vendorLength = data.readUInt32LE(offset);
    offset += 4;
    if (offset + vendorLength > data.length) return tags;
    offset += vendorLength;

    if (offset + 4 > data.length) return tags;
    const commentCount = data.readUInt32LE(offset);
    offset += 4;

    for (let i = 0; i < commentCount && offset + 4 <= data.length; i++) {
        const commentLength = data.readUInt32LE(offset);
        offset += 4;

        if (offset + commentLength > data.length) break;

        const comment = data.toString('utf8', offset, offset + commentLength);
        offset += commentLength;

        const eqIndex = comment.indexOf('=');
        if (eqIndex > 0) {
            const key = comment.substring(0, eqIndex).toUpperCase();
            const value = comment.substring(eqIndex + 1);
            if (!tags[key]) tags[key] = value;
        }
    }

    return tags;
}

/**
 * Extract cover art from a FLAC file at a known block offset.
 * Only reads the PICTURE block bytes — no streaming, no full file read.
 *
 * PICTURE block layout:
 *   4 bytes BE: picture type
 *   4 bytes BE: MIME string length
 *   N bytes:    MIME string
 *   4 bytes BE: description length
 *   N bytes:    description string
 *   4×4 bytes:  width, height, depth, num_colors
 *   4 bytes BE: picture data length
 *   N bytes:    picture data
 */
function extractFlacPicture(filePath, pictureOffset, pictureSize) {
    const fd = fs.openSync(filePath, 'r');
    try {
        const data = Buffer.alloc(pictureSize);
        fs.readSync(fd, data, 0, pictureSize, pictureOffset);

        let offset = 0;
        // const pictureType = data.readUInt32BE(offset);
        offset += 4;

        const mimeLength = data.readUInt32BE(offset);
        offset += 4;
        const mime = data.toString('ascii', offset, offset + mimeLength);
        offset += mimeLength;

        const descLength = data.readUInt32BE(offset);
        offset += 4;
        offset += descLength;

        offset += 16;

        const picDataLength = data.readUInt32BE(offset);
        offset += 4;

        const picData = data.slice(offset, offset + picDataLength);

        return { data: picData, format: mime };
    } catch (err) {
        console.error('Failed to extract FLAC picture:', err.message);
        return null;
    } finally {
        fs.closeSync(fd);
    }
}

module.exports = { parseFlacFile, extractFlacPicture };
