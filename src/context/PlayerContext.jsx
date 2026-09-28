// src/context/PlayerContext.jsx
import React, { createContext, useContext, useState, useRef, useEffect, useCallback, useMemo } from 'react';

const PlayerContext = createContext();

export const usePlayer = () => useContext(PlayerContext);

export const PlayerProvider = ({ children }) => {
    // --- STATE ---
    const getInitialSession = () => {
        try {
            const saved = localStorage.getItem('playback_session');
            if (saved) return JSON.parse(saved);
        } catch { }
        return null;
    };
    const initialSession = getInitialSession();

    const [isPlaying, setIsPlaying] = useState(false);
    const [volume, setVolume] = useState(initialSession?.volume ?? 0.5);
    const [isMuted, setIsMuted] = useState(false);
    const [prevVolume, setPrevVolume] = useState(initialSession?.volume ?? 0.5);
    const [currentTime, setCurrentTime] = useState(initialSession?.currentTime ?? 0);
    const [playQueue, setPlayQueue] = useState(initialSession?.playQueue ?? []);
    const [currentTrackIndex, setCurrentTrackIndex] = useState(initialSession?.currentTrackIndex ?? 0);
    const [isShuffle, setIsShuffle] = useState(false);
    const [repeatMode, setRepeatMode] = useState(0); // 0: None, 1: All, 2: One

    const [audioDevices, setAudioDevices] = useState([]);
    const [selectedDeviceId, setSelectedDeviceId] = useState('default');

    const [isExclusiveMode, setIsExclusiveMode] = useState(() => {
        return localStorage.getItem('exclusive_mode') === 'true';
    });
    const [hasLoadedFile, setHasLoadedFile] = useState(false);

    const [currentTrack, setCurrentTrack] = useState(initialSession?.currentTrack ?? {
        id: "default",
        title: "",
        artist: "",
        album: "",
        duration: 0,
        coverArt: null,
        src: null
    });

    const [playlists, setPlaylists] = useState(() => {
        try {
            const saved = localStorage.getItem('my_playlists');
            return saved ? JSON.parse(saved) : [];
        } catch { return []; }
    });

    const [likedSongs, setLikedSongs] = useState(() => {
        try {
            const saved = localStorage.getItem('liked_songs');
            return saved ? JSON.parse(saved) : [];
        } catch { return []; }
    });

    const audioRef = useRef(null);

    // --- PERSISTENCE ---
    useEffect(() => {
        localStorage.setItem('my_playlists', JSON.stringify(playlists));
    }, [playlists]);

    useEffect(() => {
        localStorage.setItem('liked_songs', JSON.stringify(likedSongs));
    }, [likedSongs]);


    // Save session automatically
    useEffect(() => {
        const session = { volume, playQueue, currentTrackIndex, currentTrack };
        localStorage.setItem('playback_session', JSON.stringify({ ...session, currentTime }));
    }, [volume, playQueue, currentTrackIndex, currentTrack]);

    // Throttle currentTime saves to avoid writing 4x a second
    useEffect(() => {
        const timer = setInterval(() => {
            const sessionData = localStorage.getItem('playback_session');
            if (sessionData) {
                try {
                    const session = JSON.parse(sessionData);
                    session.currentTime = currentTime;
                    localStorage.setItem('playback_session', JSON.stringify(session));
                } catch(e) {}
            }
        }, 5000);
        return () => clearInterval(timer);
    }, [currentTime]);

    // Restore session time on mount without playing
    useEffect(() => {
        if (initialSession && initialSession.currentTime > 0) {
            const timer = setTimeout(() => {
                if (audioRef.current) {
                    audioRef.current.src = initialSession.currentTrack?.src || '';
                    audioRef.current.currentTime = initialSession.currentTime;
                }
            }, 500);
            return () => clearTimeout(timer);
        }
    }, []);

    const stateRef = useRef({ currentTrack, currentTime, isPlaying, volume });
    useEffect(() => {
        stateRef.current = { currentTrack, currentTime, isPlaying, volume };
    });

    useEffect(() => {
        localStorage.setItem('exclusive_mode', isExclusiveMode);
        if (window.require) {
            const { ipcRenderer } = window.require('electron');
            const state = stateRef.current;

            if (isExclusiveMode) {
                // Switch TO MPV
                if (audioRef.current) audioRef.current.pause();

                ipcRenderer.invoke('mpv-start').then(res => {
                    if (res && res.error) console.error("MPV Start Error:", res.error);

                    ipcRenderer.invoke('mpv-set-volume', state.volume);

                    if (state.currentTrack && state.currentTrack.src) {
                        ipcRenderer.invoke('mpv-play', state.currentTrack.src, state.currentTime).then(() => {
                            if (!state.isPlaying) {
                                ipcRenderer.invoke('mpv-pause');
                            }
                        });
                    }
                });
            } else {
                // Switch TO HTML5
                ipcRenderer.invoke('mpv-stop').then(() => {
                    if (state.currentTrack && state.currentTrack.src && audioRef.current) {
                        audioRef.current.src = state.currentTrack.src;
                        audioRef.current.currentTime = state.currentTime;
                        audioRef.current.volume = state.volume;
                        if (state.isPlaying) {
                            audioRef.current.play().catch(console.error);
                        }
                    }
                });
            }
        }
    }, [isExclusiveMode]);

    // --- AUDIO CONTROL ---
    useEffect(() => {
        // Always keep HTML5 audio volume in sync as baseline
        if (audioRef.current) audioRef.current.volume = volume;
        // Additionally sync to MPV if exclusive mode is active
        if (isExclusiveMode && window.require) {
            window.require('electron').ipcRenderer.invoke('mpv-set-volume', volume).catch(() => {});
        }
        setIsMuted(volume === 0);
    }, [volume, isExclusiveMode]);

    const toggleMute = useCallback(() => {
        if (isMuted) {
            setVolume(prevVolume === 0 ? 0.5 : prevVolume);
        } else {
            setPrevVolume(volume);
            setVolume(0);
        }
    }, [isMuted, prevVolume, volume]);

    const handleSetVolume = useCallback((val) => {
        setVolume(val);
    }, []);

    // --- DEVICE MANAGEMENT (NEW) ---
    const getAudioDevices = async () => {
        try {
            if (!navigator.mediaDevices || !navigator.mediaDevices.enumerateDevices) {
                console.warn("Browser does not support enumerateDevices");
                return;
            }
            const devices = await navigator.mediaDevices.enumerateDevices();
            const audioOutputs = devices.filter(device => device.kind === 'audiooutput');
            setAudioDevices(audioOutputs);
        } catch (err) {
            console.error("Error fetching device list:", err);
        }
    };

    const setAudioOutputDevice = async (deviceId) => {
        if (audioRef.current && typeof audioRef.current.setSinkId === 'function') {
            try {
                await audioRef.current.setSinkId(deviceId);
                setSelectedDeviceId(deviceId);
            } catch (error) {
                console.error('Error in setSinkId:', error);
            }
        }
    };

    useEffect(() => {
        getAudioDevices();
        if (navigator.mediaDevices) {
            navigator.mediaDevices.ondevicechange = () => getAudioDevices();
        }
    }, []);

    // --- PLAYBACK LOGIC ---
    const playTrack = useCallback((track, startTime = 0) => {
        setCurrentTrack(track);
        setHasLoadedFile(true);
        if (isExclusiveMode) {
            if (window.require) {
                window.require('electron').ipcRenderer.invoke('mpv-play', track.src, startTime)
                    .then(() => setIsPlaying(true))
                    .catch(e => console.error("MPV Playback error:", e));
            }
        } else {
            if (audioRef.current && track.src) {
                audioRef.current.src = track.src;
                if (selectedDeviceId !== 'default' && typeof audioRef.current.setSinkId === 'function') {
                    audioRef.current.setSinkId(selectedDeviceId).catch(err => console.log(err));
                }
                audioRef.current.currentTime = startTime;
                audioRef.current.play()
                    .then(() => setIsPlaying(true))
                    .catch(e => console.error("Playback error:", e));
            }
        }
    }, [selectedDeviceId, isExclusiveMode]);

    const togglePlay = useCallback(() => {
        if (!currentTrack || !currentTrack.src) return;
        if (isExclusiveMode) {
            if (window.require) {
                if (!hasLoadedFile) {
                    playTrack(currentTrack, currentTime);
                } else {
                    window.require('electron').ipcRenderer.invoke('mpv-toggle-pause');
                    setIsPlaying(!isPlaying);
                }
            }
        } else {
            if (audioRef.current) {
                if (isPlaying) {
                    audioRef.current.pause();
                    setIsPlaying(false);
                } else {
                    audioRef.current.play()
                        .then(() => setIsPlaying(true))
                        .catch(e => console.error(e));
                }
            }
        }
    }, [isPlaying, currentTrack, isExclusiveMode]);

    const seekTrack = useCallback((time) => {
        setCurrentTime(time);
        if (isExclusiveMode) {
            if (window.require) window.require('electron').ipcRenderer.invoke('mpv-seek', time);
        } else {
            if (audioRef.current) audioRef.current.currentTime = time;
        }
    }, [isExclusiveMode]);

    const handleNext = useCallback(() => {
        if (playQueue.length <= 0) return;
        let nextIndex = currentTrackIndex + 1;
        if (isShuffle) {
            nextIndex = Math.floor(Math.random() * playQueue.length);
        } else if (nextIndex >= playQueue.length) {
            if (repeatMode === 1) nextIndex = 0;
            else { setIsPlaying(false); return; }
        }
        setCurrentTrackIndex(nextIndex);
        playTrack(playQueue[nextIndex]);
    }, [playQueue, currentTrackIndex, isShuffle, repeatMode, playTrack]);

    const handlePrev = useCallback(() => {
        if (playQueue.length <= 0) return;
        const elapsed = isExclusiveMode
            ? stateRef.current.currentTime
            : (audioRef.current?.currentTime || 0);
        if (elapsed > 3) {
            seekTrack(0);
            return;
        }
        let prevIndex = currentTrackIndex - 1;
        if (prevIndex < 0) prevIndex = playQueue.length - 1;
        setCurrentTrackIndex(prevIndex);
        playTrack(playQueue[prevIndex]);
    }, [playQueue, currentTrackIndex, playTrack, isExclusiveMode, seekTrack]);

    const handleTrackEnded = useCallback(() => {
        if (repeatMode === 2) {
            if (isExclusiveMode) {
                if (window.require) window.require('electron').ipcRenderer.invoke('mpv-seek', 0);
            } else {
                if (audioRef.current) {
                    audioRef.current.currentTime = 0;
                    audioRef.current.play();
                }
            }
        } else if (playQueue.length > 0) {
            handleNext();
        } else {
            setIsPlaying(false);
        }
    }, [repeatMode, playQueue, handleNext, isExclusiveMode]);

    // Sync MPV IPC events
    const handleTrackEndedRef = useRef();
    handleTrackEndedRef.current = handleTrackEnded;

    useEffect(() => {
        if (!window.require) return;
        const { ipcRenderer } = window.require('electron');

        const handleTimePos = (e, time) => {
            if (isExclusiveMode) setCurrentTime(time);
        };

        const handlePause = (e, paused) => {
            if (isExclusiveMode) setIsPlaying(!paused);
        };

        const handleEof = () => {
            if (isExclusiveMode && handleTrackEndedRef.current) {
                handleTrackEndedRef.current();
            }
        };

        ipcRenderer.on('mpv-time-pos', handleTimePos);
        ipcRenderer.on('mpv-pause', handlePause);
        ipcRenderer.on('mpv-eof', handleEof);

        return () => {
            ipcRenderer.removeListener('mpv-time-pos', handleTimePos);
            ipcRenderer.removeListener('mpv-pause', handlePause);
            ipcRenderer.removeListener('mpv-eof', handleEof);
        };
    }, [isExclusiveMode]);

    const startAlbumPlayback = (tracks, startIndex = 0) => {
        setPlayQueue(tracks);
        setCurrentTrackIndex(startIndex);
        playTrack(tracks[startIndex]);
    };

    const addToQueue = (track) => {
        if (playQueue.length === 0) {
            playTrack(track);
            setPlayQueue([track]);
            setCurrentTrackIndex(0);
        } else {
            setPlayQueue(prev => [...prev, track]);
        }
    };

    const removeFromQueue = (indexToRemove) => {
        setPlayQueue(prev => {
            const newQueue = prev.filter((_, index) => index !== indexToRemove);
            if (indexToRemove < currentTrackIndex) setCurrentTrackIndex(old => old - 1);
            return newQueue;
        });
    };

    // --- PLAYLIST & LIKE LOGIC ---
    const createPlaylist = (name) => {
        const newPlaylist = { id: Date.now(), name: name, tracks: [], coverArt: null };
        setPlaylists(prev => [...prev, newPlaylist]);
    };

    const addTrackToPlaylist = (playlistId, track) => {
        setPlaylists(prev => prev.map(pl => {
            if (pl.id === playlistId) {
                const exists = (pl.tracks || []).some(t => t.id === track.id);
                if (exists) return pl;
                const newTracks = [...pl.tracks, track];
                return { ...pl, tracks: newTracks, coverArt: pl.coverArt || track.coverArt };
            }
            return pl;
        }));
    };

    const removeTrackFromPlaylist = (playlistId, track) => {
        setPlaylists(prev => prev.map(pl => {
            if (pl.id === playlistId) {
                const newTracks = (pl.tracks || []).filter(t => t.id !== track.id);
                return { ...pl, tracks: newTracks };
            }
            return pl;
        }));
    };

    const addAlbumToPlaylist = (playlistId, albumTracks) => {
        setPlaylists(prev => prev.map(pl => {
            if (pl.id === playlistId) {
                const newTracks = [...pl.tracks];
                albumTracks.forEach(track => {
                    if (!newTracks.some(t => t.id === track.id)) {
                        newTracks.push(track);
                    }
                });
                return { ...pl, tracks: newTracks, coverArt: pl.coverArt || albumTracks[0]?.coverArt };
            }
            return pl;
        }));
    };

    const updatePlaylistCover = (playlistId, newCoverUrl) => {
        setPlaylists(prev => prev.map(pl => pl.id === playlistId ? { ...pl, coverArt: newCoverUrl } : pl));
    };

    const deletePlaylist = (playlistId) => {
        setPlaylists(prev => prev.filter(pl => pl.id !== playlistId));
    };

    const checkIsLiked = (track) => {
        if (!track || !track.id) return false;
        return likedSongs.some(song => song.id === track.id);
    };

    const toggleLike = (track = null) => {
        const targetTrack = track || currentTrack;
        if (!targetTrack || !targetTrack.id) return;
        if (checkIsLiked(targetTrack)) {
            setLikedSongs(prev => prev.filter(song => song.id !== targetTrack.id));
        } else {
            setLikedSongs(prev => [...prev, targetTrack]);
        }
    };

    const toggleLikeMultiple = (tracks) => {
        if (!tracks || tracks.length === 0) return;
        const allLiked = tracks.every(t => likedSongs.some(ls => ls.id === t.id));
        if (allLiked) {
            const trackIdsToRemove = new Set(tracks.map(t => t.id));
            setLikedSongs(prev => prev.filter(s => !trackIdsToRemove.has(s.id)));
        } else {
            const newSongs = tracks.filter(t => !likedSongs.some(ls => ls.id === t.id));
            setLikedSongs(prev => [...prev, ...newSongs]);
        }
    };

    // --- MEDIA SESSION API ---

    // Generate silent WAV blob URL once (cached)
    const silentSrcRef = useRef(null);
    const getSilentSrc = useCallback(() => {
        if (silentSrcRef.current) return silentSrcRef.current;
        const sampleRate = 8000;
        const numSamples = sampleRate;
        const dataSize = numSamples;
        const fileSize = 44 + dataSize;
        const buf = new ArrayBuffer(fileSize);
        const v = new DataView(buf);
        const ws = (o, s) => { for (let i = 0; i < s.length; i++) v.setUint8(o + i, s.charCodeAt(i)); };
        ws(0, 'RIFF'); v.setUint32(4, fileSize - 8, true); ws(8, 'WAVE'); ws(12, 'fmt ');
        v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, 1, true);
        v.setUint32(24, sampleRate, true); v.setUint32(28, sampleRate, true);
        v.setUint16(32, 1, true); v.setUint16(34, 8, true); ws(36, 'data'); v.setUint32(40, dataSize, true);
        for (let i = 44; i < fileSize; i++) v.setUint8(i, 128);
        silentSrcRef.current = URL.createObjectURL(new Blob([buf], { type: 'audio/wav' }));
        return silentSrcRef.current;
    }, []);

    useEffect(() => {
        if (!audioRef.current || !isExclusiveMode) return;

        if (isPlaying && currentTrack.title) {
            const el = audioRef.current;
            if (!el.src || !el.src.startsWith('blob:')) {
                el.src = getSilentSrc();
                el.loop = true;
            }
            el.volume = 0;
            el.play().catch(() => {});
        } else {
            audioRef.current.pause();
        }
    }, [isExclusiveMode, isPlaying, currentTrack.title, getSilentSrc]);

    const artworkBlobRef = useRef(null);

    useEffect(() => {
        if (!('mediaSession' in navigator) || !currentTrack.title) return;

        let cancelled = false;

        const setMeta = (artworkArr) => {
            if (cancelled) return;
            try {
                navigator.mediaSession.metadata = new MediaMetadata({
                    title: currentTrack.title,
                    artist: currentTrack.artist || 'Unknown Artist',
                    album: currentTrack.album || 'Unknown Album',
                    artwork: artworkArr,
                });
            } catch (e) { console.warn("MediaSession Metadata error", e); }
        };

        if (currentTrack.coverArt) {
            try {
                const fs = window.require('fs');
                const filePath = currentTrack.coverArt.replace(/^file:\/\//, '');
                fs.readFile(filePath, (err, data) => {
                    if (err || cancelled) {
                        if (!err) setMeta([]);
                        return;
                    }
                    const blob = new Blob([data], { type: 'image/jpeg' });
                    if (artworkBlobRef.current) URL.revokeObjectURL(artworkBlobRef.current);
                    artworkBlobRef.current = URL.createObjectURL(blob);
                    setMeta([{ src: artworkBlobRef.current, sizes: '512x512', type: 'image/jpeg' }]);
                });
            } catch (e) {
                setMeta([]);
            }
        } else {
            setMeta([]);
        }

        return () => { cancelled = true; };
    }, [currentTrack]);

    useEffect(() => {
        if ('mediaSession' in navigator) {
            navigator.mediaSession.playbackState = isPlaying ? 'playing' : 'paused';
        }
    }, [isPlaying]);

    useEffect(() => {
        if ('mediaSession' in navigator && navigator.mediaSession.setPositionState && currentTrack.duration > 0) {
            try {
                navigator.mediaSession.setPositionState({
                    duration: currentTrack.duration,
                    playbackRate: 1,
                    position: Math.min(currentTime, currentTrack.duration),
                });
            } catch (e) { }
        }
    }, [currentTime, currentTrack.duration]);

    useEffect(() => {
        if ('mediaSession' in navigator) {
            try {
                navigator.mediaSession.setActionHandler('play', () => togglePlay());
                navigator.mediaSession.setActionHandler('pause', () => togglePlay());
                navigator.mediaSession.setActionHandler('previoustrack', () => handlePrev());
                navigator.mediaSession.setActionHandler('nexttrack', () => handleNext());
                navigator.mediaSession.setActionHandler('seekto', (details) => {
                    if (details.seekTime != null) {
                        seekTrack(details.seekTime);
                    }
                });
            } catch (e) { console.warn("MediaSession Handler error", e); }
        }
    }, [togglePlay, handlePrev, handleNext, seekTrack]);

    const contextValue = useMemo(() => ({
        isPlaying, setIsPlaying, volume, setVolume, currentTime, setCurrentTime,
        currentTrack, setCurrentTrack, playQueue, setPlayQueue, isShuffle, setIsShuffle, repeatMode, setRepeatMode,
        togglePlay, handleNext, handlePrev, startAlbumPlayback,
        playlists, createPlaylist, addTrackToPlaylist, removeTrackFromPlaylist, addAlbumToPlaylist, deletePlaylist, audioRef,
        toggleLikeMultiple, likedSongs, toggleLike,
        isLiked: checkIsLiked(currentTrack),
        checkIsLiked, updatePlaylistCover, toggleMute, isMuted,
        playTrack, handleSetVolume, addToQueue, removeFromQueue, currentTrackIndex, setCurrentTrackIndex,
        audioDevices, selectedDeviceId, setAudioOutputDevice, getAudioDevices,
        isExclusiveMode, setIsExclusiveMode, seekTrack
    }), [
        isPlaying, volume, currentTime, currentTrack, playQueue, isShuffle, repeatMode,
        togglePlay, handleNext, handlePrev, startAlbumPlayback, playlists,
        likedSongs, isMuted, playTrack, handleSetVolume, addToQueue, removeFromQueue, currentTrackIndex,
        audioDevices, selectedDeviceId, isExclusiveMode, seekTrack,
        setIsPlaying, setVolume, setCurrentTime, setCurrentTrack, setPlayQueue, setIsShuffle, setRepeatMode,
        createPlaylist, addTrackToPlaylist, removeTrackFromPlaylist, addAlbumToPlaylist, deletePlaylist,
        toggleLikeMultiple, toggleLike, checkIsLiked, updatePlaylistCover, toggleMute, setCurrentTrackIndex,
        setAudioOutputDevice, getAudioDevices, setIsExclusiveMode
    ]);

    return (
        <PlayerContext.Provider value={contextValue}>
            {children}
            <audio
                ref={audioRef}
                onTimeUpdate={() => { if (!isExclusiveMode && audioRef.current) setCurrentTime(audioRef.current.currentTime); }}
                onLoadedMetadata={() => { if (!isExclusiveMode && audioRef.current) setCurrentTrack(prev => ({ ...prev, duration: audioRef.current.duration })); }}
                onEnded={handleTrackEnded}
                onPause={() => { if (!isExclusiveMode) setIsPlaying(false); }}
                onPlay={() => { if (!isExclusiveMode) setIsPlaying(true); }}
            />
        </PlayerContext.Provider>
    );
};