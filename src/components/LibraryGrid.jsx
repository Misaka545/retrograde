// src/components/LibraryGrid.jsx
import React, { memo, useState, useRef, useEffect, useLayoutEffect, forwardRef, useMemo } from 'react';
import { Play, Disc, FolderPlus, ListMusic, Trash2, CheckSquare, Square, X, Heart, ArrowUpDown, ListPlus, Check } from 'lucide-react';
import { VirtuosoGrid } from 'react-virtuoso';
import { usePlayer } from '../context/PlayerContext';
import CustomModal from './CustomModal';
import CoverImage from './CoverImage';

const gridComponents = {
    List: forwardRef(({ style, children, ...props }, ref) => (
        <div ref={ref} {...props} className="virtuoso-search-list" style={style}>
            {children}
        </div>
    )),
    Item: ({ children, ...props }) => (
        <div {...props} className="virtuoso-search-item">
            {children}
        </div>
    ),
};

const AlbumCard = memo(({ item, type, idx, onSelect, onPlay, isPlaying, selectable, selected, onToggleSelect, onContextMenu }) => {
    const isTrack = type === 'track';
    const title = isTrack ? item.title : item.name;
    const subtitle = isTrack ? item.artist : (type === 'playlist' ? `${item.tracks?.length || 0} tracks` : item.artist);

    return (
        <div
            onClick={() => {
                if (selectable) { onToggleSelect(item); return; }
                onSelect(item);
            }}
            onContextMenu={(e) => onContextMenu && onContextMenu(e, item, type)}
            className={`bg-[#161616] p-4 border transition-colors cursor-pointer group flex flex-col relative overflow-hidden ${
                selected ? 'border-[#FF6B35] bg-[#FF6B35]/10' : 'border-[#333] hover:border-[#E8C060]'
            }`}
        >
            {/* Selection checkbox */}
            {selectable && (
                <div className="absolute top-2 right-2 z-30">
                    {selected ? (
                        <CheckSquare size={20} className="text-[#FF6B35]" />
                    ) : (
                        <Square size={20} className="text-[#555] group-hover:text-[#888]" />
                    )}
                </div>
            )}

            <div className="absolute top-0 left-0 w-2 h-2 border-t border-l border-[#333] group-hover:border-[#E8C060] transition-colors"></div>

            <div className="relative aspect-square mb-4 bg-[#222] overflow-hidden border border-[#2a2a2a] flex items-center justify-center">
                <CoverImage 
                    key={item.coverArt || `${item.id || idx}-cover`}
                    src={item.coverArt} 
                    alt={title} 
                    type={type} 
                    isPlaying={isPlaying}
                    className="w-full h-full group-hover:scale-105 transition-transform duration-500"
                />

                {!selectable && (
                    <div className="absolute bottom-2 right-2 translate-y-8 opacity-0 group-hover:translate-y-0 group-hover:opacity-100 transition-all duration-300 z-20">
                        <button
                            onClick={(e) => { e.stopPropagation(); onPlay(item, type); }}
                            className="w-10 h-10 bg-[#EAEAEA] text-black flex items-center justify-center shadow-lg hover:scale-110 hover:bg-white transition-transform"
                            style={{ clipPath: 'polygon(20% 0, 100% 0, 100% 80%, 80% 100%, 0 100%, 0 20%)' }}
                            title="Play Now"
                        >
                            <Play size={20} fill="currentColor" className="ml-1" />
                        </button>
                    </div>
                )}
            </div>

            <div className="flex items-center gap-2 mb-1">
                {isTrack && <span className="text-[8px] bg-[#333] text-[#ccc] px-1 font-mono rounded-sm">TRK</span>}
                {type === 'album' && <span className="text-[8px] bg-[#333] text-[#ccc] px-1 font-mono rounded-sm">ALB</span>}
                {type === 'playlist' && <span className="text-[8px] bg-[#333] text-[#ccc] px-1 font-mono rounded-sm">PL</span>}
                <h3 className={`font-bold truncate text-sm tracking-wide flex-1 ${isPlaying ? 'text-[#FF6B35]' : 'text-white'}`}>{title}</h3>
            </div>
            <p className="text-[10px] text-[#666] font-mono truncate uppercase">{subtitle}</p>
        </div>
    );
});

const LibraryGrid = ({ albums, onSelect, onScanFolder, isSearchMode, searchResults, searchTab = 'all', onBatchDelete, scrollContainerRef }) => {
    const { startAlbumPlayback, currentTrack, addToQueue, toggleLikeMultiple, likedSongs, playlists, addAlbumToPlaylist, removeTrackFromPlaylist } = usePlayer();
    const [selectMode, setSelectMode] = useState(false);
    const [selectedAlbums, setSelectedAlbums] = useState(new Set());
    const [showDeleteModal, setShowDeleteModal] = useState(false);
    const [contextMenu, setContextMenu] = useState({ visible: false, x: 0, y: 0, album: null, type: 'album' });
    const [singleDeleteTarget, setSingleDeleteTarget] = useState(null);
    const [sortOption, setSortOptionState] = useState(() => localStorage.getItem('library_sort') || 'added_desc');
    const setSortOption = (val) => { setSortOptionState(val); localStorage.setItem('library_sort', val); };
    const [isSortOpen, setIsSortOpen] = useState(false);
    const sortRef = useRef(null);
    const contextMenuRef = useRef(null);

    useEffect(() => {
        const handleClickOutside = (e) => {
            if (sortRef.current && !sortRef.current.contains(e.target)) {
                setIsSortOpen(false);
            }
        };
        document.addEventListener('mousedown', handleClickOutside);
        return () => document.removeEventListener('mousedown', handleClickOutside);
    }, []);

    useLayoutEffect(() => {
        if (contextMenu.visible && contextMenuRef.current) {
            const rect = contextMenuRef.current.getBoundingClientRect();
            const vw = window.innerWidth;
            const vh = window.innerHeight;
            let newX = contextMenu.x;
            let newY = contextMenu.y;
            
            const bottomMargin = 104;
            if (rect.right > vw - 8) newX = vw - rect.width - 8;
            if (rect.bottom > vh - bottomMargin) newY = vh - rect.height - bottomMargin;
            
            newX = Math.max(8, newX);
            newY = Math.max(8, newY);
            
            if (newX !== contextMenu.x || newY !== contextMenu.y) {
                setContextMenu(prev => ({ ...prev, x: newX, y: newY }));
            }
        }
    }, [contextMenu.visible, contextMenu.x, contextMenu.y]);

    const sortOptions = [
        { value: 'added_desc', label: 'RECENTLY ADDED' },
        { value: 'added_asc', label: 'OLDEST ADDED' },
        { value: 'name_asc', label: 'NAME [A-Z]' },
        { value: 'name_desc', label: 'NAME [Z-A]' },
        { value: 'artist_asc', label: 'ARTIST [A-Z]' },
        { value: 'artist_desc', label: 'ARTIST [Z-A]' },
        { value: 'year_desc', label: 'NEWEST' },
        { value: 'year_asc', label: 'OLDEST' },
    ];

    const handleContextMenu = (e, album, type) => {
        if (selectMode) return;
        e.preventDefault();
        setContextMenu({
            visible: true,
            x: e.clientX,
            y: e.clientY,
            album: album,
            type: type
        });
    };

    const closeContextMenu = () => setContextMenu({ ...contextMenu, visible: false });

    const handleAddAlbumToQueue = () => {
        if (contextMenu.album && contextMenu.album.tracks) {
            contextMenu.album.tracks.forEach(track => addToQueue(track));
        }
        closeContextMenu();
    };

    const handleLikeAlbum = () => {
        if (contextMenu.album && contextMenu.album.tracks) {
            toggleLikeMultiple(contextMenu.album.tracks);
        }
        closeContextMenu();
    };

    const handleDeleteClick = () => {
        setSingleDeleteTarget(contextMenu.album);
        setShowDeleteModal(true);
        closeContextMenu();
    };

    const handlePlay = (item, type) => {
        if (type === 'track' || !item.tracks) {
            if (item.album && albums[item.album] && albums[item.album].tracks) {
                const trackIndex = albums[item.album].tracks.findIndex(t => t.id === item.id || t.title === item.title);
                if (trackIndex !== -1) {
                    startAlbumPlayback(albums[item.album].tracks, trackIndex);
                    return;
                }
            }
            startAlbumPlayback([item], 0);
        } else if (item.tracks && item.tracks.length > 0) {
            startAlbumPlayback(item.tracks, 0);
        }
    };

    const handleSearchItemClick = (item, type) => {
        if (type === 'track') {
            handlePlay(item, 'track');
        } else {
            onSelect(item);
        }
    };

    const toggleSelect = (item) => {
        setSelectedAlbums(prev => {
            const next = new Set(prev);
            if (next.has(item.name)) next.delete(item.name);
            else next.add(item.name);
            return next;
        });
    };

    const selectAll = () => {
        const all = new Set(Object.values(albums).map(a => a.name));
        setSelectedAlbums(all);
    };

    const cancelSelect = () => {
        setSelectMode(false);
        setSelectedAlbums(new Set());
    };

    const confirmBatchDelete = () => {
        if (onBatchDelete) {
            if (singleDeleteTarget) {
                onBatchDelete([singleDeleteTarget.name]);
            } else {
                onBatchDelete(Array.from(selectedAlbums));
            }
        }
        setShowDeleteModal(false);
        setSingleDeleteTarget(null);
        cancelSelect();
    };

    const albumList = useMemo(() => {
        return Object.values(albums).sort((a, b) => {
            if (sortOption === 'added_desc') return (b.addedAt || 0) - (a.addedAt || 0);
            if (sortOption === 'added_asc') return (a.addedAt || 0) - (b.addedAt || 0);
            if (sortOption === 'name_asc') return a.name.localeCompare(b.name);
            if (sortOption === 'name_desc') return b.name.localeCompare(a.name);
            if (sortOption === 'artist_asc') return (a.artist || '').localeCompare(b.artist || '');
            if (sortOption === 'artist_desc') return (b.artist || '').localeCompare(a.artist || '');
            if (sortOption === 'year_desc') return (b.year || 0) - (a.year || 0);
            if (sortOption === 'year_asc') return (a.year || 0) - (b.year || 0);
            return 0;
        });
    }, [albums, sortOption]);

    if (!isSearchMode && Object.values(albums).length === 0) {
        return (
            <div className="flex flex-col items-center justify-center h-64 border border-dashed border-[#333] bg-[#111]/50 rounded-lg">
                <FolderPlus size={48} className="text-[#333] mb-4" />
                <p className="text-lg font-bold text-[#555] tracking-widest">DATABASE_EMPTY</p>
                <button onClick={onScanFolder} className="mt-4 px-6 py-2 bg-[#222] border border-[#444] text-[#ccc] font-mono text-xs hover:bg-[#333] hover:text-white cursor-pointer transition-all">
                    INITIATE_SCAN
                </button>
            </div>
        );
    }

    const renderGrid = (items, type) => (
        <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 gap-6">
            {items.map((item, idx) => (
                <AlbumCard
                    key={`${type}-${item.id || item.name || idx}`}
                    item={item}
                    type={type}
                    idx={idx}
                    onSelect={isSearchMode ? (itm) => handleSearchItemClick(itm, type) : onSelect}
                    onPlay={handlePlay}
                    isPlaying={type === 'track' && currentTrack?.id === item.id}
                    selectable={selectMode && type === 'album' && !isSearchMode}
                    selected={selectedAlbums.has(item.name)}
                    onToggleSelect={toggleSelect}
                    onContextMenu={handleContextMenu}
                />
            ))}
        </div>
    );

    if (!isSearchMode) {
        
        return (
            <div className="animate-in fade-in slide-in-from-bottom-2 duration-300">
                {/* Header with batch actions */}
                <div className={`flex items-center justify-between mb-6 ${selectMode ? 'sticky top-0 z-40 bg-[#111]/95 backdrop-blur-md pt-6 pb-4 -mx-6 px-6 -mt-6 border-b border-[#333]' : ''}`}>
                    <h2 className="text-xl font-bold text-white font-futura tracking-widest uppercase border-l-4 border-[#FF6B35] pl-3">Library_Data</h2>
                    <div className="flex items-center gap-3">
                        {selectMode ? (
                            <>
                                <span className="text-[10px] font-mono text-[#888] tracking-wider">
                                    {selectedAlbums.size} / {albumList.length} SELECTED
                                </span>
                                <button onClick={selectAll} className="text-[10px] font-mono text-[#4FD6BE] hover:text-white tracking-wider transition-colors">
                                    SELECT_ALL
                                </button>
                                <button
                                    onClick={() => selectedAlbums.size > 0 && setShowDeleteModal(true)}
                                    disabled={selectedAlbums.size === 0}
                                    className="flex items-center gap-1 px-3 py-1 bg-[#FF6B35]/20 border border-[#FF6B35] text-[#FF6B35] text-[10px] font-mono tracking-wider hover:bg-[#FF6B35]/40 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
                                >
                                    <Trash2 size={12} /> DELETE
                                </button>
                                <button onClick={cancelSelect} className="p-1 hover:bg-[#333] text-[#888] hover:text-white rounded transition-colors">
                                    <X size={16} />
                                </button>
                            </>
                        ) : (
                            <>
                                <div className="relative" ref={sortRef}>
                                    <button 
                                        onClick={() => setIsSortOpen(!isSortOpen)}
                                        className={`flex items-center gap-2 px-3 py-1 border text-[10px] font-mono tracking-wider transition-colors cursor-pointer ${
                                            isSortOpen ? 'bg-[#333] border-[#555] text-white' : 'bg-transparent border-[#333] text-[#888] hover:border-[#555] hover:text-white'
                                        }`}
                                    >
                                        <ArrowUpDown size={12} className={isSortOpen ? "text-[#E8C060]" : "text-[#888]"} />
                                        {sortOptions.find(o => o.value === sortOption)?.label}
                                    </button>
                                    
                                    {isSortOpen && (
                                        <div className="absolute top-full right-0 mt-1 w-40 bg-[#1a1a1a] border border-[#333] shadow-2xl z-50 py-1 animate-in fade-in slide-in-from-top-2 duration-200">
                                            {sortOptions.map(opt => (
                                                <div 
                                                    key={opt.value}
                                                    onClick={() => { setSortOption(opt.value); setIsSortOpen(false); }}
                                                    className={`px-3 py-2 text-[10px] font-mono cursor-pointer transition-colors flex items-center gap-2 ${
                                                        sortOption === opt.value 
                                                            ? 'bg-[#FF6B35]/10 text-[#FF6B35] border-l-2 border-[#FF6B35]' 
                                                            : 'text-[#888] hover:bg-[#333] hover:text-white border-l-2 border-transparent'
                                                    }`}
                                                >
                                                    {opt.label}
                                                </div>
                                            ))}
                                        </div>
                                    )}
                                </div>
                                <button
                                    onClick={() => setSelectMode(true)}
                                    className="flex items-center gap-1 px-3 py-1 border border-[#333] text-[#888] text-[10px] font-mono tracking-wider hover:border-[#555] hover:text-white transition-colors"
                                >
                                    <CheckSquare size={12} /> SELECT
                                </button>
                            </>
                        )}
                    </div>
                </div>
                {renderGrid(albumList, 'album')}

                {/* Batch Delete Modal */}
                <CustomModal
                    isOpen={showDeleteModal}
                    title="WARNING: BATCH_DELETE"
                    onConfirm={confirmBatchDelete}
                    onCancel={() => setShowDeleteModal(false)}
                    confirmText="PURGE"
                >
                    <div className="font-mono text-sm text-[#ccc]">
                        TARGETS: <span className="text-[#FF6B35] font-bold">{singleDeleteTarget ? 1 : selectedAlbums.size}</span> {singleDeleteTarget ? 'ALBUM' : 'ALBUM(S)'}<br/>
                        {singleDeleteTarget && <div className="text-white mt-1 mb-2 border-l-2 border-[#FF6B35] pl-2">{singleDeleteTarget.name}</div>}
                        DATA WILL BE PERMANENTLY ERASED.
                    </div>
                </CustomModal>

                {/* Album Context Menu */}
                {contextMenu.visible && (
                    <>
                        <div className="fixed inset-0 z-[99]" onClick={closeContextMenu} onContextMenu={(e) => { e.preventDefault(); closeContextMenu(); }}></div>
                        <div ref={contextMenuRef} className="fixed bg-[#1a1a1a] border border-[#333] shadow-2xl z-[100] w-60 p-1" style={{ top: contextMenu.y, left: contextMenu.x }}>
                            <div className="px-3 py-2 text-[9px] font-bold text-[#FF6B35] border-b border-[#333] mb-1 font-mono tracking-wider">ALBUM_OPERATIONS</div>
                            
                            <button onClick={handleAddAlbumToQueue} className="w-full text-left px-3 py-2 hover:bg-[#333] text-xs text-white flex items-center gap-3 transition-colors">
                                <ListMusic size={14} className="text-[#555]" />
                                <span>ADD_ALL_TO_QUEUE</span>
                            </button>

                            <button onClick={handleLikeAlbum} className="w-full text-left px-3 py-2 hover:bg-[#333] text-xs text-white flex items-center gap-3 transition-colors">
                                {contextMenu.album && contextMenu.album.tracks && contextMenu.album.tracks.every(t => likedSongs.some(ls => ls.id === t.id)) ? (
                                    <>
                                        <Heart size={14} className="text-[#FF6B35] fill-[#FF6B35]" />
                                        <span>UNLIKE_ALL_TRACKS</span>
                                    </>
                                ) : (
                                    <>
                                        <Heart size={14} className="text-[#555]" />
                                        <span>LIKE_ALL_TRACKS</span>
                                    </>
                                )}
                            </button>

                            {(contextMenu.type === 'album' || contextMenu.type === 'playlist') && (
                                <button onClick={handleDeleteClick} className="w-full text-left px-3 py-2 hover:bg-[#333] text-xs text-[#FF6B35] flex items-center gap-3 transition-colors">
                                    <Trash2 size={14} />
                                    <span>DELETE_{contextMenu.type === 'playlist' ? 'PLAYLIST' : 'ALBUM'}</span>
                                </button>
                            )}

                            {/* ADD TO PLAYLIST sub-menu */}
                            {contextMenu.album && contextMenu.album.tracks && (
                                <>
                                    <div className="h-[1px] bg-[#333] my-1"></div>
                                    <div className="px-3 py-1 text-[8px] font-bold text-[#555] uppercase tracking-wider">ADD_TO_PLAYLIST</div>
                                    <div className="max-h-40 overflow-y-auto custom-scrollbar">
                                        {playlists.length === 0 ? <div className="px-3 py-2 text-[10px] text-[#555] italic">NO_DATA</div> : playlists.map(pl => {
                                            const allExist = contextMenu.album.tracks.every(t => (pl.tracks || []).some(pt => pt.id === t.id));
                                            return (
                                                <button 
                                                    key={pl.id} 
                                                    onClick={() => {
                                                        if (allExist) {
                                                            contextMenu.album.tracks.forEach(t => removeTrackFromPlaylist(pl.id, t));
                                                        } else {
                                                            addAlbumToPlaylist(pl.id, contextMenu.album.tracks);
                                                        }
                                                        closeContextMenu();
                                                    }} 
                                                    className={`w-full text-left px-3 py-2 hover:bg-[#333] text-xs text-white flex items-center gap-2 transition-colors`}
                                                >
                                                    <div className={`w-1 h-1 ${allExist ? 'bg-[#FF6B35]' : 'bg-[#E8C060]'}`}></div>
                                                    <span className="flex-1 truncate">{pl.name}</span>
                                                    {allExist && <Check size={12} className="text-[#FF6B35] ml-auto"/>}
                                                </button>
                                            );
                                        })}
                                    </div>
                                </>
                            )}
                        </div>
                    </>
                )}
            </div>
        );
    }

    const { tracks, albums: searchAlbums, playlists: searchPlaylists } = searchResults;
    const showTracks = searchTab === 'all' || searchTab === 'tracks';
    const showAlbums = searchTab === 'all' || searchTab === 'albums';
    const showPlaylists = searchTab === 'all' || searchTab === 'playlists';

    const scrollParent = scrollContainerRef?.current || undefined;

    const renderVirtuosoSection = (items, type, label) => {
        if (!items || items.length === 0) return null;
        return (
            <div>
                <h2 className="text-sm font-bold text-[#888] mb-4 font-mono tracking-widest uppercase border-b border-[#333] pb-2">{label} ({items.length})</h2>
                {scrollParent && items.length > 30 ? (
                    <VirtuosoGrid
                        customScrollParent={scrollParent}
                        totalCount={items.length}
                        overscan={1200}
                        components={gridComponents}
                        itemContent={(index) => {
                            const item = items[index];
                            if (!item) return null;
                            return (
                                <AlbumCard
                                    key={item.id || item.filePath || `${type}-${index}`}
                                    item={item}
                                    type={type}
                                    idx={index}
                                    onSelect={(itm) => handleSearchItemClick(itm, type)}
                                    onPlay={handlePlay}
                                    isPlaying={type === 'track' && currentTrack?.id === item.id}
                                    selectable={false}
                                    selected={false}
                                    onToggleSelect={toggleSelect}
                                    onContextMenu={handleContextMenu}
                                />
                            );
                        }}
                    />
                ) : (
                    renderGrid(items, type)
                )}
            </div>
        );
    };

    return (
        <div className="animate-in fade-in slide-in-from-bottom-2 duration-300 flex flex-col gap-10">
            {showTracks && renderVirtuosoSection(tracks, 'track', 'Tracks')}
            {showAlbums && renderVirtuosoSection(searchAlbums, 'album', 'Albums')}
            {showPlaylists && renderVirtuosoSection(searchPlaylists, 'playlist', 'Playlists')}
            {tracks.length === 0 && searchAlbums.length === 0 && searchPlaylists.length === 0 && (
                <div className="text-center py-20 text-[#555] font-mono text-xs tracking-widest">
                    NO RESULTS FOUND FOR QUERY
                </div>
            )}
        </div>
    );
};

export default LibraryGrid;