// src/App.jsx
import React, { useState, useMemo, useEffect, useRef, useCallback, useDeferredValue } from 'react';
import * as mm from 'music-metadata-browser';
import { PlayerProvider, usePlayer } from './context/PlayerContext';
import Sidebar from './components/Sidebar';
import PlayerBar from './components/PlayerBar';
import LibraryGrid from './components/LibraryGrid';
import AlbumDetail from './components/AlbumDetail';
import CustomModal from './components/CustomModal';
import FullScreenPlayer from './components/FullScreenPlayer';
import QueuePopup from './components/QueuePopup';
import TitleBar from './components/TitleBar';
import { Search, Play, Disc, ListMusic } from 'lucide-react';
import { saveAlbumToDB, getAllAlbumsFromDB, deleteAlbumFromDB, saveCoverArt, batchDeleteAlbumsFromDB } from './utils/db';
import { prewarmCoverCache } from './components/CoverImage';

const AppContent = () => {
    const [activeView, setActiveView] = useState('library');
    const [libraryAlbums, setLibraryAlbums] = useState({});
    const [selectedAlbum, setSelectedAlbum] = useState(null);
    const [isLoading, setIsLoading] = useState(false);
    const [isDeleting, setIsDeleting] = useState(false);
    const [searchQuery, setSearchQuery] = useState("");
    const deferredSearchQuery = useDeferredValue(searchQuery);
    const [uploadModal, setUploadModal] = useState({ open: false, files: [] });
    const [searchTab, setSearchTab] = useState('all'); // 'all', 'tracks', 'albums', 'playlists'
    const { likedSongs, playlists, startAlbumPlayback, currentTrack, togglePlay, handleNext, handlePrev, toggleMute, volume, handleSetVolume, isShuffle, setIsShuffle, repeatMode, setRepeatMode } = usePlayer();
    const [isFullScreen, setIsFullScreen] = useState(false);
    const [showQueue, setShowQueue] = useState(false);
    const [scrollPos, setScrollPos] = useState(0);
    const scrollRef = useRef(null);

    const [showExitModal, setShowExitModal] = useState(false);
    const [rememberExitChoice, setRememberExitChoice] = useState(false);

    useEffect(() => {
        if (window.require) {
            const { ipcRenderer } = window.require('electron');
            const handleTrayInfoRequest = () => setShowExitModal(true);
            ipcRenderer.on('request-tray-minimize-info', handleTrayInfoRequest);
            return () => {
                ipcRenderer.removeListener('request-tray-minimize-info', handleTrayInfoRequest);
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
                    const tracksWithUrls = album.tracks.map(track => ({
                        ...track,
                        coverArt: album.coverArt, coverArtFull: album.coverArtFull
                    }));
                    loadedLibrary[album.name] = { ...album, tracks: tracksWithUrls };
                }
                setLibraryAlbums(loadedLibrary);
                const coverUrls = Object.values(loadedLibrary)
                    .map(a => a.coverArt)
                    .filter(Boolean);
                prewarmCoverCache(coverUrls);
            } catch (error) { console.error("DB Load Error:", error); }
            setIsLoading(false);
        };
        loadLibrary();
    }, []);

    const handleImportMusic = (e) => {
        const files = Array.from(e.target.files).filter(f => f.type.startsWith('audio/') || f.name.endsWith('.flac'));
        if (files.length > 0) setUploadModal({ open: true, files: files });
        e.target.value = null;
    };

    const confirmUpload = async () => {
        setUploadModal({ ...uploadModal, open: false });
        setIsLoading(true);
        const files = uploadModal.files;
        const tempLibrary = { ...libraryAlbums };

        const BATCH_SIZE = 5;
        const processFile = async (file) => {
            try {
                const metadata = await mm.parseBlob(file);
                return { file, metadata };
            } catch (err) {
                console.error(err);
                return null;
            }
        };

        for (let i = 0; i < files.length; i += BATCH_SIZE) {
            const batch = files.slice(i, i + BATCH_SIZE);
            const results = await Promise.all(batch.map(processFile));

            for (const result of results) {
                if (!result) continue;
                const { file, metadata } = result;
                const albumName = metadata.common.album || "Unknown Album";
                const artist = metadata.common.artist || "Unknown Artist";

                if (!tempLibrary[albumName]) {
                    tempLibrary[albumName] = {
                        name: albumName, artist: artist,
                        coverArt: null, tracks: []
                    };
                }

                if (metadata.common.picture && metadata.common.picture.length > 0 && !tempLibrary[albumName].coverArt) {
                    const picture = metadata.common.picture[0];
                    const coverResult = saveCoverArt(albumName, picture.data, picture.format);
                    if (coverResult) {
                        tempLibrary[albumName].coverArt = coverResult.thumb;
                        tempLibrary[albumName].coverArtFull = coverResult.full;
                    }
                }

                const fileSrc = `file://${file.path.replace(/\\/g, '/')}`;
                const alreadyExists = tempLibrary[albumName].tracks.some(t => t.filePath === file.path);
                if (!alreadyExists) {
                    tempLibrary[albumName].tracks.push({
                        id: file.path + Date.now() + Math.random(), title: metadata.common.title || file.name,
                        artist, album: albumName, duration: metadata.format.duration || 0,
                        filePath: file.path, src: fileSrc, coverArt: tempLibrary[albumName].coverArt, coverArtFull: tempLibrary[albumName].coverArtFull
                    });
                }
            }
        }

        const { saveLibraryToDB } = await import('./utils/db');
        await saveLibraryToDB(tempLibrary);
        setLibraryAlbums(tempLibrary);
        setIsLoading(false);
    };
    const handleDeleteAlbum = async (albumName) => {
        await deleteAlbumFromDB(albumName);
        setLibraryAlbums(prev => {
            const newLib = { ...prev };
            delete newLib[albumName];
            return newLib;
        });
        setSelectedAlbum(null);
        setActiveView('library');
    };

    const navigateToAlbum = (album) => {
        if (scrollRef.current) setScrollPos(scrollRef.current.scrollTop);
        setSelectedAlbum(album);
        setActiveView('album-detail');
        requestAnimationFrame(() => { if (scrollRef.current) scrollRef.current.scrollTop = 0; });
    };

    const handleBackFromAlbum = useCallback(() => {
        const savedPos = scrollPos;
        setSelectedAlbum(null);
        setActiveView('library');
        requestAnimationFrame(() => {
            requestAnimationFrame(() => {
                if (scrollRef.current) scrollRef.current.scrollTop = savedPos;
            });
        });
    }, [scrollPos]);

    // Keyboard Shortcuts
    useEffect(() => {
        const handleKeyDown = (e) => {
            if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') return;

            if (e.ctrlKey) {
                switch(e.code) {
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

            switch(e.code) {
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
                    else if (uploadModal.open) setUploadModal({ ...uploadModal, open: false });
                    else if (showQueue) setShowQueue(false);
                    else if (isFullScreen) setIsFullScreen(false);
                    else if (activeView === 'album-detail') handleBackFromAlbum();
                    break;
                case 'KeyM':
                    toggleMute();
                    break;
                case 'KeyK':
                    handleSetVolume(Math.min(1, volume + 0.1));
                    break;
                case 'KeyJ':
                    handleSetVolume(Math.max(0, volume - 0.1));
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
    }, [togglePlay, handleNext, handlePrev, showExitModal, uploadModal.open, showQueue, isFullScreen, activeView, handleBackFromAlbum, toggleMute, volume, handleSetVolume, isShuffle, setIsShuffle, repeatMode, setRepeatMode]);

    const handleBatchDelete = async (albumNames) => {
        setIsDeleting(true);
        await batchDeleteAlbumsFromDB(albumNames);
        setLibraryAlbums(prev => {
            const newLib = { ...prev };
            albumNames.forEach(name => delete newLib[name]);
            return newLib;
        });
        setIsDeleting(false);
    };

    const handleOpenCurrentAlbum = () => {
        if (!currentTrack || !currentTrack.id) {
            console.log("No track playing or track has no ID");
            return;
        }

        console.log("Finding album for track:", currentTrack.title, currentTrack.id);

        const foundLibraryAlbumKey = Object.keys(libraryAlbums).find(key =>
            libraryAlbums[key].tracks.some(t => t.id === currentTrack.id)
        );

        if (foundLibraryAlbumKey) {
            console.log("Found in Library:", foundLibraryAlbumKey);
            setSelectedAlbum(libraryAlbums[foundLibraryAlbumKey]);
            setActiveView('album-detail');
            setIsFullScreen(false);
            return;
        }

        const foundPlaylist = playlists.find(pl =>
            pl.tracks.some(t => t.id === currentTrack.id)
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

        console.warn("Không tìm thấy album gốc của bài hát này.");
    };

    const likedSongsAlbum = { name: "Bài hát đã thích", artist: "User Data", coverArt: "https://t.scdn.co/images/3099b3803ad9496896c43f22fe9be8c4.png", tracks: likedSongs };

    // SEARCH LOGIC 
    const searchResults = useMemo(() => {
        if (!deferredSearchQuery.trim()) return { tracks: [], albums: [], playlists: [] };
        const query = deferredSearchQuery.toLowerCase();
        let allTracks = [];
        Object.values(libraryAlbums).forEach(alb => allTracks.push(...alb.tracks));
        playlists.forEach(pl => allTracks.push(...pl.tracks));
        likedSongs.forEach(s => allTracks.push(s));

        const uniqueTracks = Array.from(new Set(allTracks.map(t => t.id))).map(id => allTracks.find(t => t.id === id));

        return {
            tracks: uniqueTracks.filter(t => t.title.toLowerCase().includes(query) || t.artist.toLowerCase().includes(query)),
            albums: Object.values(libraryAlbums).filter(a => a.name.toLowerCase().includes(query) || a.artist.toLowerCase().includes(query)),
            playlists: playlists.filter(p => p.name.toLowerCase().includes(query))
        };
    }, [deferredSearchQuery, libraryAlbums, playlists, likedSongs]);

    return (
        <div className="h-screen w-screen bg-[#09090b] text-[#EAEAEA] font-sans overflow-hidden flex flex-col selection:bg-[#FF6B35] selection:text-black">

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
                        onUpload={handleImportMusic}
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

                        {/* Scrollable Content Area */}
                        <div ref={scrollRef} className="flex-1 overflow-y-auto custom-scrollbar relative">
                            {isLoading && <div className="text-center text-[#4FD6BE] font-mono tracking-widest mt-4 animate-pulse">SYSTEM_SCANNING...</div>}

                            {isDeleting && (
                                <div className="absolute inset-0 z-50 bg-black/70 backdrop-blur-sm flex flex-col items-center justify-center">
                                    <div className="w-12 h-12 border-2 border-[#333] border-t-[#E8C060] rounded-full animate-spin"></div>
                                    <div className="text-[#E8C060] font-mono tracking-widest mt-4 animate-pulse text-xs">DELETING_DATA...</div>
                                </div>
                            )}

                            {/* ROUTING VIEWS */}
                            {activeView === 'album-detail' && selectedAlbum ? (
                                <AlbumDetail
                                    album={selectedAlbum}
                                    onBack={handleBackFromAlbum}
                                    onDeleteAlbum={() => handleDeleteAlbum(selectedAlbum.name)}
                                />
                            ) : activeView === 'liked-songs' ? (
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
                                        <LibraryGrid albums={libraryAlbums} onSelect={navigateToAlbum} onUpload={handleImportMusic} isSearchMode={true} searchResults={searchResults} searchTab={searchTab} />
                                    </div>
                                </div>
                            ) : (
                                <div className="p-6">
                                    <LibraryGrid albums={libraryAlbums} onSelect={navigateToAlbum} onUpload={handleImportMusic} onBatchDelete={handleBatchDelete} />
                                </div>
                            )}
                        </div>
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
            <CustomModal isOpen={uploadModal.open} title="CONFIRM_UPLOAD" onConfirm={confirmUpload} onCancel={() => setUploadModal({ ...uploadModal, open: false })} confirmText="EXECUTE">
                <div className="font-mono text-sm text-[#ccc]">DETECTED <span className="text-[#4FD6BE] font-bold">{uploadModal.files.length}</span> FILES.<br />INITIALIZE IMPORT SEQUENCE?</div>
            </CustomModal>

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