import React, { useState, useEffect, useRef } from 'react';
import { Play, Heart, MoreHorizontal, Disc, Plus, Check, Trash2, Image as ImageIcon, Upload, ListPlus } from 'lucide-react'; 
import { usePlayer } from '../context/PlayerContext';
import { formatTime } from '../utils/timeUtils';
import CustomModal from './CustomModal';

const AlbumDetail = ({ album, onBack, onDeleteAlbum }) => { 
  const { startAlbumPlayback, currentTrack, playlists, addTrackToPlaylist, toggleLike, checkIsLiked, deletePlaylist, updatePlaylistCover, toggleLikeMultiple, likedSongs, addToQueue } = usePlayer();
  
  const [contextMenu, setContextMenu] = useState({ visible: false, x: 0, y: 0, track: null });
  const [showSettingsMenu, setShowSettingsMenu] = useState(false);
  const [isDeletePlaylistModalOpen, setIsDeletePlaylistModalOpen] = useState(false);
  const [isDeleteAlbumModalOpen, setIsDeleteAlbumModalOpen] = useState(false);

  const handleAddToQueue = () => {
      if (contextMenu.track) {
          addToQueue(contextMenu.track);
      }
      closeContextMenu();
  };
  
  const menuRef = useRef(null);
  const settingsRef = useRef(null);
  const fileInputRef = useRef(null);

  const isUserPlaylist = playlists.some(pl => pl.id === album.id);
  const isUploadedAlbum = !isUserPlaylist && album.name !== "Bài hát đã thích";
  const isAlbumLiked = album.tracks.length > 0 && album.tracks.every(t => likedSongs.some(ls => ls.title === t.title));

  useEffect(() => {
    if (contextMenu.visible) document.body.style.overflow = 'hidden';
    else document.body.style.overflow = 'unset';
    return () => { document.body.style.overflow = 'unset'; }
  }, [contextMenu.visible]);

  useEffect(() => {
      const handleClickOutside = (e) => {
          if (settingsRef.current && !settingsRef.current.contains(e.target)) setShowSettingsMenu(false);
      };
      document.addEventListener('mousedown', handleClickOutside);
      return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  if (!album) return null;

  const albumTracks = album.tracks.map(t => ({ ...t, coverArt: t.coverArt || album.coverArt }));
  const handlePlay = (index) => startAlbumPlayback(albumTracks, index);
  const handleContextMenu = (e, track) => { e.preventDefault(); setContextMenu({ visible: true, x: e.clientX, y: e.clientY, track: track }); };
  const closeContextMenu = () => setContextMenu({ ...contextMenu, visible: false });
  const handleToggleLikeSingle = () => { if (contextMenu.track) toggleLike(contextMenu.track); closeContextMenu(); }
  const handleAddToPlaylist = (playlistId) => { if (contextMenu.track) addTrackToPlaylist(playlistId, contextMenu.track); closeContextMenu(); };
  const handleDeletePlaylistClick = () => { setShowSettingsMenu(false); setIsDeletePlaylistModalOpen(true); };
  const confirmDeletePlaylist = () => { deletePlaylist(album.id); setIsDeletePlaylistModalOpen(false); if (onBack) setTimeout(() => onBack(), 100); };
  const handleDeleteAlbumClick = () => { setShowSettingsMenu(false); setIsDeleteAlbumModalOpen(true); };
  const confirmDeleteAlbum = () => { if (onDeleteAlbum) onDeleteAlbum(); setIsDeleteAlbumModalOpen(false); };
  const handleCoverUpload = (e) => { const file = e.target.files[0]; if (file) { const objectUrl = URL.createObjectURL(file); updatePlaylistCover(album.id, objectUrl); setShowSettingsMenu(false); } };
  const handleLikeAlbum = () => { toggleLikeMultiple(album.tracks); };
  const isContextMenuTrackLiked = contextMenu.track ? checkIsLiked(contextMenu.track) : false;

  // Tri-color bar component
  const TriColorBar = () => (
    <div className="flex h-[3px] w-48 mb-4">
      <div className="w-1/3 h-full bg-[#FF6B35]"></div>
      <div className="w-1/3 h-full bg-[#E8C060]"></div>
      <div className="w-1/3 h-full bg-[#4FD6BE]"></div>
    </div>
  );

  return (
    <div className="animate-in fade-in duration-300 pb-20 relative bg-gradient-to-b from-[#1a1a1a] to-[#111]">
        {/* Background Tech Elements */}
        <div className="absolute right-0 top-0 w-1/2 h-96 opacity-10 pointer-events-none" style={{ backgroundImage: 'repeating-linear-gradient(45deg, #333 0, #333 1px, transparent 0, transparent 50%)', backgroundSize: '10px 10px' }}></div>

        {/* HEADER */}
        <div className="p-8 flex gap-8 items-end relative">
            {/* Album Art Frame */}
            <div className="w-56 h-56 relative flex-shrink-0 group">
                <div className="absolute inset-0 border border-[#444] translate-x-2 translate-y-2"></div>
                <div className="absolute inset-0 border border-[#666] -translate-x-1 -translate-y-1 z-10 bg-[#222]">
                    {album.coverArt ? <img src={album.coverArt} className="w-full h-full object-cover" /> : <div className="w-full h-full flex items-center justify-center"><Disc size={48} className="text-[#555]"/></div>}
                    {/* Tech Lines Overlay */}
                    <div className="absolute inset-0 border-[0.5px] border-white/20 m-2 pointer-events-none flex flex-col justify-between p-1">
                        <div className="flex justify-between"><span className="w-1 h-1 bg-white/50"></span><span className="w-1 h-1 bg-white/50"></span></div>
                        <div className="flex justify-between"><span className="w-1 h-1 bg-white/50"></span><span className="w-1 h-1 bg-white/50"></span></div>
                    </div>
                </div>
                {isUserPlaylist && <div onClick={() => fileInputRef.current.click()} className="absolute inset-0 -translate-x-1 -translate-y-1 z-20 bg-black/60 opacity-0 group-hover:opacity-100 flex flex-col items-center justify-center transition-opacity cursor-pointer text-white"><ImageIcon size={32} className="mb-2"/><span className="text-xs font-bold">CHANGE_IMG</span></div>}
            </div>

            {/* Info */}
            <div className="flex flex-col z-10 w-full">
                <div className="flex items-center gap-3 mb-1">
                    <span className="bg-[#4FD6BE]/10 border border-[#4FD6BE] text-[#4FD6BE] text-[9px] font-bold px-2 py-0.5 tracking-widest uppercase">{isUserPlaylist ? 'PLAYLIST' : 'ALBUM_DATA'}</span>
                    <div className="h-[1px] flex-1 bg-[#333]"></div>
                    <span className="text-[#E8C060] font-mono text-[10px]">{album.tracks.length} TRACKS</span>
                </div>
                
                <h1 className="text-6xl font-bold tracking-tighter text-white mb-2 leading-none uppercase">{album.name}</h1>
                <TriColorBar />

                <div className="flex items-center gap-4 text-xs font-medium text-[#888] uppercase tracking-wide">
                    <span className="text-white">{album.artist || "UNKNOWN_ARTIST"}</span>
                    <span className="w-1 h-1 bg-[#555] rotate-45"></span>
                    <span>FLAC / 24bit</span>
                </div>
            </div>
        </div>

        {/* ACTION BAR */}
        <div className="px-8 py-2 flex items-center gap-6 border-b border-[#333]">
            <button onClick={() => handlePlay(0)} className="w-12 h-10 bg-[#EAEAEA] text-black flex items-center justify-center hover:bg-white hover:shadow-[0_0_15px_rgba(255,255,255,0.3)] transition-all" style={{ clipPath: 'polygon(15% 0, 100% 0, 100% 85%, 85% 100%, 0 100%, 0 15%)' }}>
                <Play size={20} fill="currentColor" className="ml-1" />
            </button>
            {album.name !== "Bài hát đã thích" && (
                <button onClick={handleLikeAlbum} className="hover:scale-110 transition-transform">
                    <Heart size={28} className={isAlbumLiked ? 'text-[#FF6B35] fill-[#FF6B35]' : 'text-[#555] hover:text-white'} />
                </button>
            )}
            <div className="relative" ref={settingsRef}>
                <MoreHorizontal size={28} className={`text-[#555] hover:text-white cursor-pointer ${showSettingsMenu ? 'text-white' : ''}`} onClick={() => setShowSettingsMenu(!showSettingsMenu)} />
                {showSettingsMenu && (
                    <div className="absolute top-10 left-0 bg-[#1a1a1a] border border-[#333] shadow-xl z-30 w-48 p-1">
                        {isUserPlaylist && (
                            <>
                                <button onClick={() => fileInputRef.current.click()} className="w-full text-left px-3 py-2 text-xs text-[#ccc] hover:bg-[#333] flex items-center gap-2"><Upload size={14}/> CHANGE_COVER</button>
                                <button onClick={handleDeletePlaylistClick} className="w-full text-left px-3 py-2 text-xs text-[#ccc] hover:bg-[#333] flex items-center gap-2"><Trash2 size={14}/> DELETE_PLAYLIST</button>
                            </>
                        )}
                        {isUploadedAlbum && <button onClick={handleDeleteAlbumClick} className="w-full text-left px-3 py-2 text-xs text-[#ccc] hover:bg-[#333] flex items-center gap-2"><Trash2 size={14}/> DELETE_ALBUM</button>}
                    </div>
                )}
            </div>
            <input type="file" accept="image/*" ref={fileInputRef} onChange={handleCoverUpload} className="hidden" />
        </div>

        {/* TRACKLIST GRID */}
        <div className="px-8 mt-4">
            <div className="grid grid-cols-[50px_1fr_80px] gap-4 py-2 border-b border-[#333] text-[9px] tracking-[0.2em] text-[#555] uppercase mb-2 font-mono">
                <div className="text-center">#</div>
                <div>Operation_Log</div>
                <div className="text-right">Duration</div>
            </div>
            {albumTracks.map((track, i) => {
                const isActive = currentTrack.title === track.title;
                const isContextMenuActive = contextMenu.visible && contextMenu.track?.title === track.title;
                return (
                    <div key={i} onClick={() => handlePlay(i)} onContextMenu={(e) => handleContextMenu(e, track)}
                        className={`group grid grid-cols-[50px_1fr_80px] gap-4 py-3 cursor-pointer border-b border-[#1a1a1a] items-center transition-all relative overflow-hidden ${isActive ? 'bg-[#FF6B35]/10' : 'hover:bg-[#ffffff]/5'} ${isContextMenuActive ? 'bg-white/10' : ''}`}>
                        <div className={`absolute left-0 top-0 bottom-0 w-[2px] ${isActive ? 'bg-[#FF6B35]' : 'bg-[#4FD6BE] -translate-x-full group-hover:translate-x-0'} transition-transform`}></div>
                        <div className={`text-center font-mono ${isActive ? 'text-[#FF6B35]' : 'text-[#444] group-hover:text-[#4FD6BE]'}`}>
                            {isActive ? <img src="https://open.spotifycdn.com/cdn/images/equaliser-animated-green.f93a2bf4.gif" className="w-3 h-3 mx-auto opacity-50 grayscale" /> : `0${i+1}`}
                        </div>
                        <div className="flex flex-col">
                            <span className={`text-sm font-bold ${isActive ? 'text-[#FF6B35]' : 'text-[#ccc] group-hover:text-white'} transition-colors`}>{track.title}</span>
                            <span className="text-[10px] text-[#444] group-hover:text-[#666] font-mono uppercase tracking-wider">{track.artist}</span>
                        </div>
                        <div className="text-center font-mono text-xs text-[#555] group-hover:text-white">{formatTime(track.duration)}</div>
                    </div>
                );
            })}
        </div>

        {/* CONTEXT MENU */}
        {contextMenu.visible && (
            <>
                <div className="fixed inset-0 z-[99]" onClick={closeContextMenu} onContextMenu={(e) => { e.preventDefault(); closeContextMenu(); }}></div>
                <div className="fixed bg-[#1a1a1a] border border-[#333] shadow-2xl z-[100] w-60 p-1" style={{ top: contextMenu.y, left: contextMenu.x }}>
                    <div className="px-3 py-2 text-[9px] font-bold text-[#4FD6BE] border-b border-[#333] mb-1 font-mono tracking-wider">COMMAND_MENU</div>
                    <button onClick={handleToggleLikeSingle} className="w-full text-left px-3 py-2 hover:bg-[#333] text-xs text-white flex items-center gap-3">
                        <Heart size={14} className={isContextMenuTrackLiked ? "text-[#FF6B35] fill-[#FF6B35]" : "text-[#555]"} />
                        <span>{isContextMenuTrackLiked ? "REMOVE_FAVORITE" : "ADD_TO_FAVORITE"}</span>
                    </button>
                    <button 
                            onClick={handleAddToQueue}
                            className="w-full text-left px-3 py-2 hover:bg-[#333] text-xs text-white flex items-center gap-3"
                        >
                            <ListPlus size={14} className="text-[#555]" />
                            <span className="font-small">ADD_TO_QUEUE</span>
                    </button>
                    <div className="h-[1px] bg-[#333] my-1"></div>
                    <div className="px-3 py-1 text-[8px] font-bold text-[#555] uppercase tracking-wider">ADD_TO_PLAYLIST</div>
                    <div className="max-h-40 overflow-y-auto custom-scrollbar">
                        {playlists.length === 0 ? <div className="px-3 py-2 text-[10px] text-[#555] italic">NO_DATA</div> : playlists.map(pl => {
                            const exists = pl.tracks.some(t => t.title === contextMenu.track?.title);
                            return (
                                <button key={pl.id} onClick={() => !exists && handleAddToPlaylist(pl.id)} className={`w-full text-left px-3 py-2 hover:bg-[#333] text-xs text-white flex items-center gap-2 ${exists ? 'opacity-50 cursor-not-allowed' : ''}`}>
                                    <div className="w-1 h-1 bg-[#E8C060]"></div> {pl.name} {exists && <Check size={12} className="text-[#FF6B35] ml-auto"/>}
                                </button>
                            );
                        })}
                    </div>
                </div>
            </>
        )}

        {/* MODALS */}
        <CustomModal isOpen={isDeletePlaylistModalOpen} title="WARNING: DELETE_PLAYLIST" onConfirm={confirmDeletePlaylist} onCancel={() => setIsDeletePlaylistModalOpen(false)} confirmText="CONFIRM">
            <div className="font-mono text-sm text-[#ccc]">TARGET: <span className="text-white font-bold">{album.name}</span><br/>ACTION CANNOT BE UNDONE.</div>
        </CustomModal>
        <CustomModal isOpen={isDeleteAlbumModalOpen} title="WARNING: PURGE_DATA" onConfirm={confirmDeleteAlbum} onCancel={() => setIsDeleteAlbumModalOpen(false)} confirmText="PURGE">
            <div className="font-mono text-sm text-[#ccc]">TARGET: <span className="text-white font-bold">{album.name}</span><br/>DATA WILL BE PERMANENTLY ERASED.</div>
        </CustomModal>
    </div>
  );
};

export default AlbumDetail;