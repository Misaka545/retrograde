// src/components/LibraryGrid.jsx
import React, { memo, useState } from 'react';
import { Play, Disc, FolderPlus, ListMusic, Trash2, CheckSquare, Square, X, Heart } from 'lucide-react';
import { usePlayer } from '../context/PlayerContext';
import CustomModal from './CustomModal';
import CoverImage from './CoverImage';

const AlbumCard = memo(({ item, type, idx, onSelect, onPlay, isPlaying, selectable, selected, onToggleSelect, onContextMenu }) => {
    const isTrack = type === 'track';
    const title = isTrack ? item.title : item.name;
    const subtitle = isTrack ? item.artist : (type === 'playlist' ? `${item.tracks?.length || 0} bài` : item.artist);

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
                            title="Phát ngay"
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

const LibraryGrid = ({ albums, onSelect, onUpload, isSearchMode, searchResults, searchTab = 'all', onBatchDelete }) => {
    const { startAlbumPlayback, currentTrack, addToQueue, toggleLikeMultiple, likedSongs } = usePlayer();
    const [selectMode, setSelectMode] = useState(false);
    const [selectedAlbums, setSelectedAlbums] = useState(new Set());
    const [showDeleteModal, setShowDeleteModal] = useState(false);
    const [contextMenu, setContextMenu] = useState({ visible: false, x: 0, y: 0, album: null, type: 'album' });
    const [singleDeleteTarget, setSingleDeleteTarget] = useState(null);

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
            startAlbumPlayback([item], 0);
        } else if (item.tracks && item.tracks.length > 0) {
            startAlbumPlayback(item.tracks, 0);
        }
    };

    const handleSearchItemClick = (item, type) => {
        onSelect(item);
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

    if (!isSearchMode && Object.values(albums).length === 0) {
        return (
            <div className="flex flex-col items-center justify-center h-64 border border-dashed border-[#333] bg-[#111]/50 rounded-lg">
                <FolderPlus size={48} className="text-[#333] mb-4" />
                <p className="text-lg font-bold text-[#555] tracking-widest">DATABASE_EMPTY</p>
                <label className="mt-4 px-6 py-2 bg-[#222] border border-[#444] text-[#ccc] font-mono text-xs hover:bg-[#333] hover:text-white cursor-pointer transition-all">
                    INITIATE_SCAN <input type="file" webkitdirectory="true" directory="" multiple onChange={onUpload} className="hidden" />
                </label>
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
        const albumList = Object.values(albums);
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
                            <button
                                onClick={() => setSelectMode(true)}
                                className="flex items-center gap-1 px-3 py-1 border border-[#333] text-[#888] text-[10px] font-mono tracking-wider hover:border-[#555] hover:text-white transition-colors"
                            >
                                <CheckSquare size={12} /> SELECT
                            </button>
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
                        <div className="fixed bg-[#1a1a1a] border border-[#333] shadow-2xl z-[100] w-60 p-1" style={{ top: contextMenu.y, left: contextMenu.x }}>
                            <div className="px-3 py-2 text-[9px] font-bold text-[#FF6B35] border-b border-[#333] mb-1 font-mono tracking-wider">ALBUM_OPERATIONS</div>
                            
                            <button onClick={handleAddAlbumToQueue} className="w-full text-left px-3 py-2 hover:bg-[#333] text-xs text-white flex items-center gap-3 transition-colors">
                                <ListMusic size={14} className="text-[#555]" />
                                <span>ADD_ALL_TO_QUEUE</span>
                            </button>

                            <button onClick={handleLikeAlbum} className="w-full text-left px-3 py-2 hover:bg-[#333] text-xs text-white flex items-center gap-3 transition-colors">
                                {contextMenu.album && contextMenu.album.tracks && contextMenu.album.tracks.every(t => likedSongs.some(ls => ls.title === t.title)) ? (
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
                        </div>
                    </>
                )}
            </div>
        );
    }

    const { tracks, albums: searchAlbums, playlists } = searchResults;
    const showTracks = searchTab === 'all' || searchTab === 'tracks';
    const showAlbums = searchTab === 'all' || searchTab === 'albums';
    const showPlaylists = searchTab === 'all' || searchTab === 'playlists';

    return (
        <div className="animate-in fade-in slide-in-from-bottom-2 duration-300 flex flex-col gap-10">
            {showTracks && tracks.length > 0 && (
                <div>
                    <h2 className="text-sm font-bold text-[#888] mb-4 font-mono tracking-widest uppercase border-b border-[#333] pb-2">Tracks ({tracks.length})</h2>
                    {renderGrid(tracks, 'track')}
                </div>
            )}
            {showAlbums && searchAlbums.length > 0 && (
                <div>
                    <h2 className="text-sm font-bold text-[#888] mb-4 font-mono tracking-widest uppercase border-b border-[#333] pb-2">Albums ({searchAlbums.length})</h2>
                    {renderGrid(searchAlbums, 'album')}
                </div>
            )}
            {showPlaylists && playlists.length > 0 && (
                <div>
                    <h2 className="text-sm font-bold text-[#888] mb-4 font-mono tracking-widest uppercase border-b border-[#333] pb-2">Playlists ({playlists.length})</h2>
                    {renderGrid(playlists, 'playlist')}
                </div>
            )}
            {tracks.length === 0 && searchAlbums.length === 0 && playlists.length === 0 && (
                <div className="text-center py-20 text-[#555] font-mono text-xs tracking-widest">
                    NO RESULTS FOUND FOR QUERY
                </div>
            )}
        </div>
    );
};

export default LibraryGrid;