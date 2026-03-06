// src/components/LibraryGrid.jsx
import React from 'react';
import { Play, Disc, FolderPlus, ListMusic } from 'lucide-react';
import { usePlayer } from '../context/PlayerContext';

const LibraryGrid = ({ albums, onSelect, onUpload, isSearchMode, searchResults, searchTab = 'all' }) => {
    const { startAlbumPlayback, currentTrack } = usePlayer();

    const handlePlayClick = (e, item, type) => {
        e.stopPropagation();
        if (type === 'track') {
            startAlbumPlayback([item], 0);
        } else if (item.tracks && item.tracks.length > 0) {
            startAlbumPlayback(item.tracks, 0);
        }
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

    const renderItem = (item, type, idx) => {
        const isTrack = type === 'track';
        const title = isTrack ? item.title : item.name;
        const subtitle = isTrack ? item.artist : (type === 'playlist' ? `${item.tracks?.length || 0} bài` : item.artist);

        // Check if this track is currently playing
        const isPlaying = isTrack && currentTrack?.id === item.id;

        return (
            <div
                key={`${type}-${idx}`}
                onClick={() => isTrack ? handlePlayClick({ stopPropagation: () => { } }, item, 'track') : onSelect(item)}
                className="bg-[#161616] p-4 border border-[#333] hover:border-[#E8C060] transition-colors cursor-pointer group flex flex-col relative overflow-hidden"
            >
                {/* Tech Corner */}
                <div className="absolute top-0 left-0 w-2 h-2 border-t border-l border-[#333] group-hover:border-[#E8C060] transition-colors"></div>

                <div className="relative aspect-square mb-4 bg-[#222] overflow-hidden border border-[#2a2a2a] flex items-center justify-center">
                    {item.coverArt ? (
                        <img src={item.coverArt} className="w-full h-full object-cover group-hover:scale-105 transition-transform duration-500" />
                    ) : (
                        type === 'playlist' ? <ListMusic size={32} className="text-[#555]" /> : <Disc className={`p-8 text-[#333] w-full h-full ${isPlaying ? 'animate-spin-slow' : ''}`} />
                    )}

                    {/* --- NÚT PLAY --- */}
                    <div className="absolute bottom-2 right-2 translate-y-8 opacity-0 group-hover:translate-y-0 group-hover:opacity-100 transition-all duration-300 z-20">
                        <button
                            onClick={(e) => handlePlayClick(e, item, type)}
                            className="w-10 h-10 bg-[#EAEAEA] text-black flex items-center justify-center shadow-lg hover:scale-110 hover:bg-white transition-transform"
                            style={{ clipPath: 'polygon(20% 0, 100% 0, 100% 80%, 80% 100%, 0 100%, 0 20%)' }}
                            title="Phát ngay"
                        >
                            <Play size={20} fill="currentColor" className="ml-1" />
                        </button>
                    </div>
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
    };

    const renderGrid = (items, type) => (
        <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 gap-6">
            {items.map((item, idx) => renderItem(item, type, idx))}
        </div>
    );

    if (!isSearchMode) {
        return (
            <div className="animate-in fade-in duration-300">
                <h2 className="text-xl font-bold text-white mb-6 font-futura tracking-widest uppercase border-l-4 border-[#FF6B35] pl-3">Library_Data</h2>
                {renderGrid(Object.values(albums), 'album')}
            </div>
        );
    }

    // Search Results Rendering
    const { tracks, albums: searchAlbums, playlists } = searchResults;

    const showTracks = searchTab === 'all' || searchTab === 'tracks';
    const showAlbums = searchTab === 'all' || searchTab === 'albums';
    const showPlaylists = searchTab === 'all' || searchTab === 'playlists';

    return (
        <div className="animate-in fade-in duration-300 flex flex-col gap-10">
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