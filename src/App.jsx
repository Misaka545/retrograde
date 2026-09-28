// src/App.jsx
import React, { useState, useMemo, useEffect, useLayoutEffect, useRef, useCallback, useDeferredValue } from 'react';
import { PlayerProvider, usePlayer } from './context/PlayerContext';
import Sidebar from './components/Sidebar';
import PlayerBar from './components/PlayerBar';
import LibraryGrid from './components/LibraryGrid';
import AlbumDetail from './components/AlbumDetail';
import CustomModal from './components/CustomModal';
import FullScreenPlayer from './components/FullScreenPlayer';
import QueuePopup from './components/QueuePopup';
import TitleBar from './components/TitleBar';
import TerminalToast from './components/TerminalToast';
import { Search, Play, Disc, ListMusic } from 'lucide-react';
import { saveAlbumToDB, getAllAlbumsFromDB, deleteAlbumFromDB, saveCoverArt, batchDeleteAlbumsFromDB } from './utils/db';

const AppContent = () => {
    const [activeView, setActiveView] = useState('library');
    const [libraryAlbums, setLibraryAlbums] = useState({});
    const [selectedAlbum, setSelectedAlbum] = useState(null);
    const [isLoading, setIsLoading] = useState(true);
    const [initialLoad, setInitialLoad] = useState(true);
    const [isDeleting, setIsDeleting] = useState(false);
    const [searchQuery, setSearchQuery] = useState("");
    const deferredSearchQuery = useDeferredValue(searchQuery);
    const [searchTab, setSearchTab] = useState('all'); // 'all', 'tracks', 'albums', 'playlists'
    const { 
        currentTrack, setCurrentTrack, togglePlay, 
        playQueue, setPlayQueue,
        isShuffle, setIsShuffle, 
        repeatMode, setRepeatMode, 
        playlists, audioRef, setIsPlaying,
        handleNext, handlePrev, toggleMute, volume, handleSetVolume,
        likedSongs, startAlbumPlayback
    } = usePlayer();
    const [isFullScreen, setIsFullScreen] = useState(false);
    const [showQueue, setShowQueue] = useState(false);
    const [scrollPos, setScrollPos] = useState(0);
    const scrollRef = useRef(null);

    const [showExitModal, setShowExitModal] = useState(false);
    const [rememberExitChoice, setRememberExitChoice] = useState(false);

    const [scanProgress, setScanProgress] = useState({ phase: '', scanned: 0, total: 0, currentFile: '' });

    useEffect(() => {
        if (window.require) {
            const { ipcRenderer } = window.require('electron');
            const handleTrayInfoRequest = () => setShowExitModal(true);
            ipcRenderer.on('request-tray-minimize-info', handleTrayInfoRequest);

            const handleScanProgress = (_e, data) => {
                setScanProgress(data);
            };
            ipcRenderer.on('scan-progress', handleScanProgress);

            const handleScanComplete = (_e, data) => {
                if (data.library) {
                    const lib = data.library;
                    for (const albumName in lib) {
                        const album = lib[albumName];
                        album.tracks = (album.tracks || []).map(track => ({
                            ...track,
                            coverArt: album.coverArt,
                            coverArtFull: album.coverArtFull,
                        }));
                    }
                    setLibraryAlbums(lib);
                }
                setIsLoading(false);
                setScanProgress({ phase: '', scanned: 0, total: 0, currentFile: '' });
            };
            ipcRenderer.on('scan-complete', handleScanComplete);

            return () => {
                ipcRenderer.removeListener('request-tray-minimize-info', handleTrayInfoRequest);
                ipcRenderer.removeListener('scan-progress', handleScanProgress);
                ipcRenderer.removeListener('scan-complete', handleScanComplete);
            };
        }
    }, []);

    const handleExitChoice = (shouldMinimize) => {
        setShowExitModal(false);
        if (window.require) {
            window.require('electron').ipcRenderer.send('tray-minimize-response', {
                shouldMinimize,
                rememberChoice: rememberExitChoice
            });
        }
    };

    useEffect(() => {
        const loadLibrary = async () => {
            setIsLoading(true);
            try {
                const storedAlbums = await getAllAlbumsFromDB();
                const loadedLibrary = {};
                for (const album of storedAlbums) {
                    const tracksWithUrls = (album.tracks || []).map(track => ({
                        ...track,
                        coverArt: album.coverArt, coverArtFull: album.coverArtFull
                    }));
                    loadedLibrary[album.name] = { ...album, tracks: tracksWithUrls };
                }
                setLibraryAlbums(loadedLibrary);

                if (window.require) {
                    window.require('electron').ipcRenderer.invoke('regenerate-thumbnails')
                        .then(r => r.regenerated > 0 && console.log(`[Migration] Regenerated ${r.regenerated} thumbnails`));
                }
            } catch (error) { console.error("DB Load Error:", error); }
            setIsLoading(false);
            setInitialLoad(false);
        };
        loadLibrary();
    }, []);

    const handleScanFolder = useCallback(async () => {
        if (!window.require) return;
        const { ipcRenderer } = window.require('electron');
        setIsLoading(true);
        setScanProgress({ phase: 'discovering', scanned: 0, total: 0, currentFile: 'Opening folder dialog...' });
        try {
            const result = await ipcRenderer.invoke('scan-folder');
            if (result.cancelled) {
                setIsLoading(false);
                setScanProgress({ phase: '', scanned: 0, total: 0, currentFile: '' });
            }
        } catch (err) {
            console.error('Scan folder error:', err);
            setIsLoading(false);
            setScanProgress({ phase: '', scanned: 0, total: 0, currentFile: '' });
        }
    }, []);

    const handleScanFiles = useCallback(async () => {
        if (!window.require) return;
        const { ipcRenderer } = window.require('electron');
        setIsLoading(true);
        setScanProgress({ phase: 'discovering', scanned: 0, total: 0, currentFile: 'Opening file dialog...' });
        try {
            const result = await ipcRenderer.invoke('scan-files');
            if (result.cancelled) {
                setIsLoading(false);
                setScanProgress({ phase: '', scanned: 0, total: 0, currentFile: '' });
            }
        } catch (err) {
            console.error('Scan files error:', err);
            setIsLoading(false);
            setScanProgress({ phase: '', scanned: 0, total: 0, currentFile: '' });
        }
    }, []);
    const handleDeleteAlbum = async (albumName) => {
        await deleteAlbumFromDB(albumName);
        setLibraryAlbums(prev => {
            const newLib = { ...prev };
            delete newLib[albumName];
            return newLib;
        });

        if (currentTrack && currentTrack.album === albumName) {
            if (audioRef && audioRef.current) {
                audioRef.current.pause();
                audioRef.current.src = "";
            }
            setIsPlaying(false);
            setCurrentTrack({ id: "default", title: "", artist: "", album: "", duration: 0, coverArt: null, src: null });
        }
        setPlayQueue(prev => prev.filter(track => track.album !== albumName));

        setSelectedAlbum(null);
        setActiveView('library');
    };

    const navigateToAlbum = (album) => {
        if (!album) return;
        let targetAlbum = album;
        if (!targetAlbum.tracks && targetAlbum.album && libraryAlbums[targetAlbum.album]) {
            targetAlbum = libraryAlbums[targetAlbum.album];
        }
        if (scrollRef.current) setScrollPos(scrollRef.current.scrollTop);
        setSelectedAlbum(targetAlbum);
        setActiveView('album-detail');
        requestAnimationFrame(() => { if (scrollRef.current) scrollRef.current.scrollTop = 0; });
    };

    const pendingScrollRestore = useRef(null);

    const handleBackFromAlbum = useCallback(() => {
        pendingScrollRestore.current = scrollPos;
        setSelectedAlbum(null);
        setActiveView('library');
    }, [scrollPos]);

    useLayoutEffect(() => {
        if (activeView === 'library' && pendingScrollRestore.current !== null) {
            if (scrollRef.current) {
                scrollRef.current.scrollTop = pendingScrollRestore.current;
            }
            pendingScrollRestore.current = null;
        }
    }, [activeView]);

    // Keyboard Shortcuts
    useEffect(() => {
        const handleKeyDown = (e) => {
            if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') return;

            if (e.ctrlKey) {
                switch (e.code) {
                    case 'KeyS':
                        e.preventDefault();
                        setIsShuffle(!isShuffle);
                        break;
                    case 'KeyR':
                        e.preventDefault();
                        setRepeatMode((repeatMode + 1) % 3);
                        break;
                }
                return;
            }

            switch (e.code) {
                case 'Space':
                    e.preventDefault();
                    togglePlay();
                    break;
                case 'ArrowRight':
                    handleNext();
                    break;
                case 'ArrowLeft':
                    handlePrev();
                    break;
                case 'Escape':
                    if (showExitModal) setShowExitModal(false);
                    else if (showQueue) setShowQueue(false);
                    else if (isFullScreen) setIsFullScreen(false);
                    else if (activeView === 'album-detail') handleBackFromAlbum();
                    break;
                case 'KeyM':
                    toggleMute();
                    break;
                case 'KeyK':
                    handleSetVolume(Math.min(1, Math.round(volume * 15 + 1) / 15));
                    break;
                case 'KeyJ':
                    handleSetVolume(Math.max(0, Math.round(volume * 15 - 1) / 15));
                    break;
                case 'KeyF':
                    setIsFullScreen(!isFullScreen);
                    break;
                default:
                    break;
            }
        };

        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [togglePlay, handleNext, handlePrev, showExitModal, showQueue, isFullScreen, activeView, handleBackFromAlbum, toggleMute, volume, handleSetVolume, isShuffle, setIsShuffle, repeatMode, setRepeatMode]);

    const handleBatchDelete = async (albumNames) => {
        setIsDeleting(true);
        await batchDeleteAlbumsFromDB(albumNames);
        setLibraryAlbums(prev => {
            const newLib = { ...prev };
            albumNames.forEach(name => delete newLib[name]);
            return newLib;
        });

        if (currentTrack && albumNames.includes(currentTrack.album)) {
            if (audioRef && audioRef.current) {
                audioRef.current.pause();
                audioRef.current.src = "";
            }
            setIsPlaying(false);
            setCurrentTrack({ id: "default", title: "", artist: "", album: "", duration: 0, coverArt: null, src: null });
        }
        setPlayQueue(prev => prev.filter(track => !albumNames.includes(track.album)));

        setIsDeleting(false);
    };

    const handleOpenCurrentAlbum = () => {
        if (!currentTrack || !currentTrack.id) {
            console.log("No track playing or track has no ID");
            return;
        }

        console.log("Finding album for track:", currentTrack.title, currentTrack.id);

        const foundLibraryAlbumKey = Object.keys(libraryAlbums).find(key =>
            (libraryAlbums[key]?.tracks || []).some(t => t.id === currentTrack.id)
        );

        if (foundLibraryAlbumKey) {
            console.log("Found in Library:", foundLibraryAlbumKey);
            setSelectedAlbum(libraryAlbums[foundLibraryAlbumKey]);
            setActiveView('album-detail');
            setIsFullScreen(false);
            return;
        }

        const foundPlaylist = playlists.find(pl =>
            (pl?.tracks || []).some(t => t.id === currentTrack.id)
        );

        if (foundPlaylist) {
            console.log("Found in Playlist:", foundPlaylist.name);
            setSelectedAlbum(foundPlaylist);
            setActiveView('album-detail');
            setIsFullScreen(false);
            return;
        }

        const isLiked = likedSongs.some(t => t.id === currentTrack.id);
        if (isLiked) {
            console.log("Found in Liked Songs");
            setActiveView('liked-songs');
            setIsFullScreen(false);
            return;
        }

        if (currentTrack.album && libraryAlbums[currentTrack.album]) {
            console.log("Found by Album Name Fallback");
            setSelectedAlbum(libraryAlbums[currentTrack.album]);
            setActiveView('album-detail');
            setIsFullScreen(false);
            return;
        }

        console.warn("Original album not found for this track.");
    };

    const likedSongsAlbum = useMemo(() => ({ name: "Liked Songs", artist: "User Data", coverArt: "https://t.scdn.co/images/3099b3803ad9496896c43f22fe9be8c4.png", tracks: likedSongs }), [likedSongs]);

    // SEARCH LOGIC 
    const searchResults = useMemo(() => {
        if (!deferredSearchQuery.trim()) return { tracks: [], albums: [], playlists: [] };
        const query = deferredSearchQuery.toLowerCase();
        let allTracks = [];
        Object.values(libraryAlbums).forEach(alb => {
            if (alb?.tracks) allTracks.push(...alb.tracks);
        });
        playlists.forEach(pl => {
            if (pl?.tracks) allTracks.push(...pl.tracks);
        });
        likedSongs.forEach(s => {
            if (s) allTracks.push(s);
        });

        const seen = new Map();
        allTracks.filter(Boolean).forEach(t => { if (!seen.has(t.id)) seen.set(t.id, t); });
        const uniqueTracks = Array.from(seen.values());

        return {
            tracks: uniqueTracks.filter(t => t && (t.title?.toLowerCase().includes(query) || t.artist?.toLowerCase().includes(query))),
            albums: Object.values(libraryAlbums).filter(a => a && (a.name?.toLowerCase().includes(query) || a.artist?.toLowerCase().includes(query))),
            playlists: playlists.filter(p => p && p.name?.toLowerCase().includes(query))
        };
    }, [deferredSearchQuery, libraryAlbums, playlists, likedSongs]);

    const handleDragOver = (e) => {
        e.preventDefault();
        e.stopPropagation();
    };

    const handleDrop = async (e) => {
        e.preventDefault();
        e.stopPropagation();
        
        if (!window.require) return;
        
        const files = Array.from(e.dataTransfer.files);
        const paths = files.map(f => f.path).filter(Boolean);
        
        if (paths.length > 0) {
            setIsLoading(true);
            const { ipcRenderer } = window.require('electron');
            try {
                const data = await ipcRenderer.invoke('scan-dropped-paths', paths);
                if (!data.cancelled && data.library) {
                    const lib = data.library;
                    for (const albumName in lib) {
                        const album = lib[albumName];
                        album.tracks = album.tracks.map(track => ({
                            ...track,
                            coverArt: album.coverArt,
                            coverArtFull: album.coverArtFull,
                        }));
                    }
                    setLibraryAlbums(lib);
                }
            } catch (err) {
                console.error("Drop import failed:", err);
            }
            setIsLoading(false);
            setScanProgress({ phase: '', scanned: 0, total: 0, currentFile: '' });
        }
    };

    return (
        <div 
            className="h-screen w-screen bg-[#09090b] text-[#EAEAEA] font-sans overflow-hidden flex flex-col selection:bg-[#FF6B35] selection:text-black"
            onDragOver={handleDragOver}
            onDragEnter={handleDragOver}
            onDrop={handleDrop}
        >

            {/* 1. TitleBar */}
            <div className="flex-shrink-0 z-50">
                <TitleBar />
            </div>

            {/* 2. Main Content */}
            <div className="flex-1 flex min-h-0 relative">

                {/* Background Effects */}
                <div className="absolute inset-0 z-0 pointer-events-none opacity-20"
                    style={{ backgroundImage: `linear-gradient(rgba(255,255,255,0.03) 1px, transparent 1px), linear-gradient(90deg, rgba(255,255,255,0.03) 1px, transparent 1px)`, backgroundSize: '32px 32px' }}>
                </div>
                <div className="absolute inset-0 z-0 pointer-events-none bg-[radial-gradient(circle_at_center,transparent_0%,#000000_100%)] opacity-80"></div>

                {/* Layout */}
                <div className="flex-1 flex z-10 p-3 gap-3 h-full pb-24">

                    <Sidebar
                        libraryAlbums={libraryAlbums}
                        onScanFolder={handleScanFolder}
                        onScanFiles={handleScanFiles}
                        onViewChange={(view) => { setActiveView(view); if (view !== 'search') setSearchQuery(""); }}
                        onAlbumSelect={navigateToAlbum}
                    />

                    <div className="flex-1 bg-[#111] border border-[#333] relative flex flex-col overflow-hidden">
                        {/* Top decoration line */}
                        <div className="h-[2px] w-full flex flex-shrink-0">
                            <div className="w-1/3 bg-[#FF6B35]"></div>
                            <div className="w-1/3 bg-[#E8C060]"></div>
                            <div className="w-1/3 bg-[#4FD6BE]"></div>
                        </div>

                        {/* Startup Loading Screen */}
                        {initialLoad && (
                            <div className="absolute inset-0 z-[60] bg-[#0e0e10] flex flex-col items-center justify-center startup-loader">
                                {/* Orrery */}
                                <div className="relative w-36 h-36 mb-6 flex items-center justify-center">
                                    {/* Orbit 3 — outer */}
                                    <div className="absolute w-36 h-36 rounded-full border border-white/[0.06]"></div>
                                    <div className="absolute w-36 h-36 rounded-full animate-[orbit_20s_linear_infinite]">
                                        <div className="absolute -top-[3px] left-1/2 -translate-x-1/2 w-1.5 h-1.5 rounded-full bg-[#4FD6BE]"></div>
                                    </div>
                                    {/* Orbit 2 — mid */}
                                    <div className="absolute w-24 h-24 rounded-full border border-white/[0.08]"></div>
                                    <div className="absolute w-24 h-24 rounded-full animate-[orbit_12s_linear_infinite_reverse]">
                                        <div className="absolute -top-[3px] left-1/2 -translate-x-1/2 w-2 h-2 rounded-full bg-[#E8C060]"></div>
                                    </div>
                                    {/* Orbit 1 — inner */}
                                    <div className="absolute w-14 h-14 rounded-full border border-white/[0.1]"></div>
                                    <div className="absolute w-14 h-14 rounded-full animate-[orbit_5s_linear_infinite]">
                                        <div className="absolute -top-[2px] left-1/2 -translate-x-1/2 w-1.5 h-1.5 rounded-full bg-[#FF6B35]"></div>
                                    </div>
                                    {/* Central star */}
                                    <div className="w-3 h-3 rounded-full bg-[#FF6B35]"></div>
                                </div>
                                <div className="font-mono text-xs tracking-[0.4em] text-[#888] uppercase mb-2">RETROGRADE</div>
                                <div className="font-mono text-[10px] tracking-widest text-[#555] animate-pulse">LOADING_LIBRARY...</div>
                            </div>
                        )}

                        {/* Scrollable Content Area */}
                        <div ref={scrollRef} className="flex-1 overflow-y-auto custom-scrollbar relative" style={{ overflowAnchor: 'none' }}>

                            {isDeleting && (
                                <div className="absolute inset-0 z-50 bg-black/70 backdrop-blur-sm flex flex-col items-center justify-center">
                                    <div className="w-12 h-12 border-2 border-[#333] border-t-[#E8C060] rounded-full animate-spin"></div>
                                    <div className="text-[#E8C060] font-mono tracking-widest mt-4 animate-pulse text-xs">DELETING_DATA...</div>
                                </div>
                            )}

                            {/* ROUTING VIEWS */}
                            {activeView === 'album-detail' && selectedAlbum ? (() => {
                                const livePlaylist = playlists.find(pl => pl.id === selectedAlbum.id);
                                const liveAlbum = livePlaylist || selectedAlbum;
                                return (
                                    <AlbumDetail
                                        album={liveAlbum}
                                        onBack={handleBackFromAlbum}
                                        onDeleteAlbum={() => handleDeleteAlbum(selectedAlbum.name)}
                                    />
                                );
                            })() : activeView === 'liked-songs' ? (
                                <div className="h-full animate-in fade-in slide-in-from-bottom-2 duration-300">
                                    {likedSongs.length > 0 ? (
                                        <AlbumDetail album={likedSongsAlbum} onBack={() => setActiveView('library')} />
                                    ) : (
                                        <div className="flex flex-col items-center justify-center h-full text-[#555]">
                                            <h2 className="text-2xl font-bold text-white mb-2 font-futura tracking-widest uppercase">No Data Found</h2>
                                            <p className="font-mono text-xs">MARK TRACKS AS 'FAVORITE' TO POPULATE</p>
                                        </div>
                                    )}
                                </div>
                            ) : activeView === 'search' ? (
                                <div className="animate-in fade-in slide-in-from-bottom-2 duration-300 min-h-full bg-[#111]">
                                    <div className="sticky top-0 z-30 bg-[#0e0e10] border-b border-[#333] px-6 py-4 shadow-xl">
                                        <div className="relative max-w-md">
                                            <Search className="absolute left-3 top-1/2 -translate-y-1/2 text-[#555]" size={20} />
                                            <input
                                                type="text" placeholder="SEARCH_DATABASE..." value={searchQuery} onChange={(e) => setSearchQuery(e.target.value)} autoFocus
                                                className="w-full bg-[#1a1a1a] text-white rounded-none border border-[#444] py-3 pl-10 pr-4 outline-none focus:border-[#4FD6BE] placeholder:text-[#555] font-mono text-sm shadow-inner transition-colors"
                                            />
                                        </div>
                                        {searchQuery.trim() && (
                                            <div className="flex gap-6 mt-4 text-xs font-bold tracking-widest text-[#555]">
                                                {['all', 'tracks', 'albums', 'playlists'].map(tab => (
                                                    <button key={tab} onClick={() => setSearchTab(tab)} className={`uppercase hover:text-white transition-all pb-1 border-b-2 ${searchTab === tab ? 'text-[#FF6B35] border-[#FF6B35]' : 'border-transparent hover:border-[#333]'}`}>{tab}</button>
                                                ))}
                                            </div>
                                        )}
                                    </div>
                                    <div className="p-6 pb-10">
                                        <LibraryGrid albums={libraryAlbums} onSelect={navigateToAlbum} onScanFolder={handleScanFolder} isSearchMode={true} searchResults={searchResults} searchTab={searchTab} scrollContainerRef={scrollRef} />
                                    </div>
                                </div>
                            ) : (
                                <div className="p-6">
                                    <LibraryGrid albums={libraryAlbums} onSelect={navigateToAlbum} onScanFolder={handleScanFolder} onBatchDelete={handleBatchDelete} />
                                </div>
                            )}
                        </div>

                        {/* Floating Overlays inside main view */}
                        {isLoading && (
                            <div className="absolute bottom-6 right-6 z-50 bg-[#0e0e10]/95 border border-[#333] shadow-2xl rounded p-4 w-80 animate-in slide-in-from-bottom-5 fade-in duration-300 pointer-events-none">
                                <div className="flex items-center gap-3 mb-3">
                                    <div className="w-4 h-4 border-2 border-[#4FD6BE] border-t-transparent rounded-full animate-spin"></div>
                                    <span className="text-[#4FD6BE] font-mono tracking-widest text-xs uppercase font-bold">
                                        {scanProgress.phase === 'discovering' ? 'DISCOVERING_FILES...' :
                                         scanProgress.phase === 'scanning' ? `SCANNING_METADATA [${scanProgress.scanned}/${scanProgress.total}]` :
                                         scanProgress.phase === 'covers' ? `EXTRACTING_COVERS [${scanProgress.scanned}/${scanProgress.total}]` :
                                         'SYSTEM_SCANNING...'}
                                    </span>
                                </div>
                                {scanProgress.total > 0 && (
                                    <div className="w-full h-[3px] bg-[#222] rounded overflow-hidden">
                                        <div
                                            className="h-full bg-gradient-to-r from-[#4FD6BE] to-[#FF6B35] transition-all duration-300"
                                            style={{ width: `${Math.round((scanProgress.scanned / scanProgress.total) * 100)}%` }}
                                        ></div>
                                    </div>
                                )}
                                {scanProgress.currentFile && (
                                    <div className="text-[10px] text-[#888] font-mono mt-2 truncate" title={scanProgress.currentFile}>
                                        {scanProgress.currentFile}
                                    </div>
                                )}
                            </div>
                        )}
                    </div>
                </div>
            </div>

            {/* 3. PlayerBar */}
            <PlayerBar
                onOpenAlbum={handleOpenCurrentAlbum}
                onToggleFullScreen={() => setIsFullScreen(true)}
                onToggleQueue={() => setShowQueue(!showQueue)}
            />

            {/* Overlays */}
            {showQueue && <QueuePopup onClose={() => setShowQueue(false)} />}
            {isFullScreen && <FullScreenPlayer onClose={() => setIsFullScreen(false)} />}
            <TerminalToast />
            {/* Exit/Minimize Promt */}
            <CustomModal
                isOpen={showExitModal}
                title="BACKGROUND_PLAY"
                onConfirm={() => handleExitChoice(true)}
                onCancel={() => handleExitChoice(false)}
                confirmText="MINIMIZE"
                cancelText="QUIT"
            >
                <div className="font-mono text-sm text-[#ccc] mb-4 space-y-4">
                    <p>Keep music playing in the background?</p>
                    <p className="text-xs text-[#888]">If you choose MINIMIZE, the player will stay active in your system tray when closed. You can toggle this setting anytime from the tray icon right-click menu.</p>
                </div>
                <label className="flex items-center gap-3 cursor-pointer mt-6 border-t border-[#333] pt-4 group">
                    <input
                        type="checkbox"
                        checked={rememberExitChoice}
                        onChange={(e) => setRememberExitChoice(e.target.checked)}
                        className="w-4 h-4 rounded-sm border-[#555] bg-[#222] checked:bg-[#FF6B35] cursor-pointer appearance-none relative
                      before:content-[''] before:absolute before:inset-0 before:bg-[url('data:image/svg+xml;utf8,<svg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 24 24%22 fill=%22none%22 stroke=%22black%22 stroke-width=%223%22 stroke-linecap=%22round%22 stroke-linejoin=%22round%22><polyline points=%2220 6 9 17 4 12%22></polyline></svg>')] before:bg-no-repeat before:bg-center before:bg-[length:12px] checked:before:block before:hidden border border-solid"
                    />
                    <span className="text-[11px] font-mono tracking-widest text-[#888] uppercase select-none group-hover:text-white transition-colors">
                        Remember my choice
                    </span>
                </label>
            </CustomModal>
        </div>
    );
};

const App = () => { return (<PlayerProvider><AppContent /></PlayerProvider>) };
export default App;