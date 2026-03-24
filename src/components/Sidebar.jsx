import React, { useState, useRef, useEffect } from 'react';
import { Home, Search, FolderPlus, Heart, Plus, ListMusic, Disc, Folder, FileAudio, Terminal, X } from 'lucide-react';
import { usePlayer } from '../context/PlayerContext';
import CustomModal from './CustomModal';
import defaultEggImage from '../assets/kristen.png'; 


const eggImage = defaultEggImage || "https://images.unsplash.com/photo-1446776811953-b23d57bd21aa?q=80&w=2072&auto=format&fit=crop";

const Sidebar = ({ libraryAlbums, onUpload, onViewChange, onAlbumSelect }) => {
  const [filterMode, setFilterMode] = useState('albums');
  const { playlists, createPlaylist, likedSongs } = usePlayer();
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [newPlaylistName, setNewPlaylistName] = useState("");
  const [showUploadMenu, setShowUploadMenu] = useState(false);
  
  // --- MORSE CODE STATE ---
  const [morseBuffer, setMorseBuffer] = useState("");
  const [showEasterEgg, setShowEasterEgg] = useState(false);
  const [activeIndex, setActiveIndex] = useState(null); // 0 (Teal), 1 (Yellow), 2 (Orange)
  
  const folderInputRef = useRef(null);
  const fileInputRef = useRef(null);
  const uploadMenuRef = useRef(null);

  // Morse Refs
  const resetTimeout = useRef(null);
  
  // THE PASSCODE: LONETRAIL (.-.. --- -. . - .-. .- .. .-..)
  const TARGET_CODE = ".-.. --- -. . - .-. .- .. .-..";

  useEffect(() => {
      const handleClickOutside = (e) => {
          if (uploadMenuRef.current && !uploadMenuRef.current.contains(e.target)) setShowUploadMenu(false);
      };
      document.addEventListener('mousedown', handleClickOutside);
      return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  const handleCreateClick = () => { setNewPlaylistName(""); setIsModalOpen(true); };
  
  const confirmCreatePlaylist = () => {
      if (newPlaylistName.trim() !== "") { createPlaylist(newPlaylistName); setFilterMode('playlists'); setIsModalOpen(false); }
  };
  
  const triggerUpload = (type) => {
      setShowUploadMenu(false);
      if (type === 'folder' && folderInputRef.current) folderInputRef.current.click();
      else if (type === 'file' && fileInputRef.current) fileInputRef.current.click();
  };

  // --- MORSE CODE LOGIC ---
  const handleBarClick = (index, char) => {
      setActiveIndex(index);
      setTimeout(() => setActiveIndex(null), 150);

      setMorseBuffer(prev => {
          const newBuffer = prev + char;
         
          if (newBuffer.includes(TARGET_CODE)) {
               triggerEasterEgg();
               return "";
          }
          return newBuffer;
      });

      if (resetTimeout.current) clearTimeout(resetTimeout.current);
      resetTimeout.current = setTimeout(() => {
          setMorseBuffer("");
      }, 3000);
  };

  const triggerEasterEgg = () => {
      setShowEasterEgg(true);
      setMorseBuffer("");
      if (resetTimeout.current) clearTimeout(resetTimeout.current);
  };

  return (
    <>
        <div className="w-64 flex flex-col gap-3 h-full hidden md:flex relative">
            {/* LOGO AREA */}
            <div className="h-16 bg-[#111] border border-[#333] flex flex-col justify-center relative overflow-hidden group cursor-default">
                <div className="absolute top-0 right-0 p-1"><div className="w-1.5 h-1.5 bg-[#4FD6BE] rounded-full animate-pulse-tech"></div></div>
                <div className="px-4 flex items-center gap-3">
                    <div className="p-1.5 border border-[#FF6B35] rounded-[1px]">
                        <Terminal size={16} color="#FF6B35" />
                    </div>
                    <div>
                        <div className="font-bold tracking-[0.2em] text-xs text-white">[PROJECT_LT] RL09</div>
                        <div className="text-[8px] text-[#4FD6BE] font-mono tracking-wider">TERMINAL_V1</div>
                    </div>
                </div>
                <div className="absolute bottom-0 left-0 w-0 h-[2px] bg-[#FF6B35] group-hover:w-full transition-all duration-500"></div>
            </div>

            {/* UNIFIED MENU BOX */}
            <div className="flex-1 bg-[#0e0e10]/90 border border-[#333] backdrop-blur-sm p-0 flex flex-col overflow-hidden relative">
                {/* Tech Deco Lines */}
                <div className="absolute top-4 right-2 w-8 h-[1px] bg-[#333]"></div>
                
                {/* 1. NAVIGATION */}
                <div className="p-4 pb-2 border-b border-[#222]">
                    <div className="text-[9px] text-[#4FD6BE] font-mono mb-2 flex items-center gap-2 select-none">
                        <span className="w-2 h-[1px] bg-[#4FD6BE]"></span> SYSTEM_NAV
                    </div>
                    {/* Items */}
                    {[
                        { label: 'TRANG CHỦ', icon: Home, action: () => onViewChange('library') },
                        { label: 'TÌM KIẾM', icon: Search, action: () => onViewChange('search') }
                    ].map((item, idx) => (
                        <div key={idx} onClick={item.action} className="flex items-center justify-between p-2 hover:bg-white/5 cursor-pointer group border-l-2 border-transparent hover:border-[#FF6B35] transition-all mb-1">
                            <div className="flex items-center gap-3">
                                <item.icon size={16} className="text-[#555] group-hover:text-white transition-colors"/>
                                <span className="text-xs font-bold tracking-widest text-[#999] group-hover:text-white transition-colors">{item.label}</span>
                            </div>
                        </div>
                    ))}

                    {/* UPLOAD DROPDOWN */}
                    <div className="relative" ref={uploadMenuRef}>
                        <div onClick={() => setShowUploadMenu(!showUploadMenu)} className={`flex items-center justify-between p-2 hover:bg-white/5 cursor-pointer group border-l-2 border-transparent hover:border-[#FF6B35] transition-all mb-1 ${showUploadMenu ? 'bg-white/5 border-[#FF6B35]' : ''}`}>
                            <div className="flex items-center gap-3">
                                <FolderPlus size={16} className="text-[#555] group-hover:text-white transition-colors" />
                                <span className="text-xs font-bold tracking-widest text-[#999] group-hover:text-white transition-colors">THÊM NHẠC</span>
                            </div>
                        </div>
                        {showUploadMenu && (
                            <div className="absolute top-full left-2 right-0 bg-[#1a1a1a] border border-[#333] z-20 shadow-xl">
                                <button onClick={() => triggerUpload('folder')} className="w-full text-left px-3 py-2 text-[10px] text-[#ccc] hover:bg-[#333] tracking-wider border-b border-[#222]">SCAN_FOLDER</button>
                                <button onClick={() => triggerUpload('file')} className="w-full text-left px-3 py-2 text-[10px] text-[#ccc] hover:bg-[#333] tracking-wider">SCAN_FILES</button>
                            </div>
                        )}
                    </div>
                    <input type="file" webkitdirectory="true" directory="" multiple ref={folderInputRef} onChange={onUpload} className="hidden" />
                    <input type="file" multiple accept="audio/*,.flac" ref={fileInputRef} onChange={onUpload} className="hidden" />

                    <div onClick={() => onViewChange('liked-songs')} className="flex items-center justify-between p-2 hover:bg-white/5 cursor-pointer group border-l-2 border-transparent hover:border-[#FF6B35] transition-all">
                        <div className="flex items-center gap-3">
                            <Heart size={16} className="text-[#555] group-hover:text-white transition-colors" />
                            <span className="text-xs font-bold tracking-widest text-[#999] group-hover:text-white transition-colors">YÊU THÍCH</span>
                        </div>
                    </div>
                </div>

                {/* 2. DATA LIST (Playlists/Albums) */}
                <div className="flex-1 flex flex-col overflow-hidden">
                    <div className="px-4 py-3 flex items-center justify-between">
                        <div className="text-[9px] text-[#E8C060] font-mono flex items-center gap-2 select-none">
                            <span className="w-2 h-[1px] bg-[#E8C060]"></span> DATA_BANK
                        </div>
                        <Plus size={14} className="text-[#555] hover:text-white cursor-pointer" onClick={handleCreateClick} title="CREATE_LIST"/>
                    </div>
                    
                    {/* Filters */}
                    <div className="flex gap-1 px-4 mb-2">
                        {['playlists', 'albums'].map(mode => (
                            <button 
                                key={mode} 
                                onClick={() => setFilterMode(mode)}
                                className={`px-2 py-1 text-[9px] uppercase tracking-wider font-mono border ${filterMode === mode ? 'border-[#E8C060] text-[#E8C060] bg-[#E8C060]/10' : 'border-[#333] text-[#555] hover:border-[#666]'}`}
                            >
                                {mode}
                            </button>
                        ))}
                    </div>

                    <div className="flex-1 overflow-y-auto custom-scrollbar px-2 pb-2">
                         {filterMode === 'playlists' && (
                            <>
                                <div onClick={() => onViewChange('liked-songs')} className="flex items-center gap-3 p-2 hover:bg-[#ffffff]/5 cursor-pointer border border-transparent hover:border-[#333] transition-colors group">
                                    <div className="w-8 h-8 bg-[#1a1a1a] flex items-center justify-center border border-[#333] group-hover:border-[#E8C060] transition-colors">
                                        <Heart size={12} className="text-[#E8C060]" />
                                    </div>
                                    <div className="flex-1 min-w-0">
                                        <h4 className="text-[11px] font-bold text-[#ccc] group-hover:text-white truncate">FAVORITES</h4>
                                        <p className="text-[9px] font-mono text-[#555] truncate">{likedSongs.length} UNITS</p>
                                    </div>
                                </div>
                                {playlists.map((pl) => (
                                    <div key={pl.id} onClick={() => onAlbumSelect(pl)} className="flex items-center gap-3 p-2 hover:bg-[#ffffff]/5 cursor-pointer border border-transparent hover:border-[#333] transition-colors group">
                                        <div className="w-8 h-8 bg-[#1a1a1a] flex items-center justify-center border border-[#333] group-hover:border-[#E8C060] overflow-hidden">
                                            {pl.coverArt ? <img src={pl.coverArt} className="w-full h-full object-cover" /> : <ListMusic size={12} className="text-[#555]" />}
                                        </div>
                                        <div className="flex-1 min-w-0">
                                            <h4 className="text-[11px] font-bold text-[#ccc] group-hover:text-white truncate">{pl.name}</h4>
                                            <p className="text-[9px] font-mono text-[#555] truncate">{pl.tracks.length} UNITS</p>
                                        </div>
                                    </div>
                                ))}
                            </>
                        )}
                        {filterMode === 'albums' && Object.values(libraryAlbums).map((album, idx) => (
                            <div key={idx} onClick={() => onAlbumSelect(album)} className="flex items-center gap-3 p-2 hover:bg-[#ffffff]/5 cursor-pointer border border-transparent hover:border-[#333] transition-colors group">
                                <div className="w-8 h-8 bg-[#1a1a1a] flex items-center justify-center border border-[#333] group-hover:border-[#E8C060] overflow-hidden">
                                    {album.coverArt ? <img src={album.coverArt} className="w-full h-full object-cover" /> : <Disc size={12} className="text-[#555]" />}
                                </div>
                                <div className="flex-1 min-w-0">
                                    <h4 className="text-[11px] font-bold text-[#ccc] group-hover:text-white truncate">{album.name}</h4>
                                    <p className="text-[9px] font-mono text-[#555] truncate">{album.artist}</p>
                                </div>
                            </div>
                        ))}
                    </div>
                </div>

                {/* 3. VISUALIZER DECO (EASTER EGG TRIGGER) */}
                <div 
                    className="h-10 border-t border-[#333] bg-[#0a0a0a] select-none flex items-end justify-between px-3 pb-2 gap-1 relative"
                    title="INPUT: [TEAL=.] [GOLD=SPACE] [ORANGE=-]"
                    // THÊM DÒNG NÀY:
                    style={{ WebkitAppRegion: 'no-drag' }} 
                >
                    {/* Loop 12 bars */}
                    {[25, 45, 20, 55, 30, 60, 15, 40, 25, 50, 20, 35].map((height, i) => {
                        const group = i % 3; 
                        
                        let activeColor = "";
                        let inputChar = "";
                         if (group === 0) {
                            activeColor = "#4FD6BE"; inputChar = ".";
                        } else if (group === 1) {
                            activeColor = "#E8C060"; inputChar = " ";
                        } else {
                            activeColor = "#FF6B35"; inputChar = "-";
                        }

                        const isActive = activeIndex === i;

                        return (
                            <div 
                                key={i}
                                className="flex-1 h-full flex items-end justify-center cursor-pointer group"
                                style={{ WebkitAppRegion: 'no-drag' }}
                                onMouseDown={() => handleBarClick(i, inputChar)}
                            >
                                <div 
                                    className="w-full transition-all duration-75"
                                    style={{ 
                                        backgroundColor: isActive ? activeColor : '#333',
                                        height: isActive ? '80%' : `${height}%`,
                                        // boxShadow: isActive ? `0 0 10px ${activeColor}` : 'none'
                                    }}
                                ></div>
                            </div>
                        );
                    })}
                </div>
            </div>
        </div>

        <CustomModal isOpen={isModalOpen} title="NEW_PLAYLIST_ENTRY" onConfirm={confirmCreatePlaylist} onCancel={() => setIsModalOpen(false)} confirmText="INITIALIZE">
            <input 
                type="text" placeholder="ENTER_DESIGNATION..." value={newPlaylistName} onChange={(e) => setNewPlaylistName(e.target.value)} autoFocus
                className="w-full bg-[#111] border border-[#333] text-white p-3 font-mono text-sm outline-none focus:border-[#E8C060] placeholder:text-[#444]"
                onKeyDown={(e) => e.key === 'Enter' && confirmCreatePlaylist()}
            />
        </CustomModal>

        {/* --- EASTER EGG MODAL --- */}
        {showEasterEgg && (
            <div className="fixed inset-0 z-[200] flex items-center justify-center bg-black/95 animate-in fade-in duration-1000">
                <div className="relative max-w-lg w-full p-1 bg-[#1a1a1a] border border-[#4FD6BE] shadow-[0_0_50px_rgba(79,214,190,0.3)] mb-24">
                    
                    {/* Corner Decos */}
                    <div className="absolute -top-2 -left-2 w-4 h-4 border-t-2 border-l-2 border-[#FF6B35]"></div>
                    <div className="absolute -bottom-2 -right-2 w-4 h-4 border-b-2 border-r-2 border-[#FF6B35]"></div>

                    {/* Content Container */}
                    <div className="bg-[#09090b] relative overflow-hidden">
                        
                        {/* Header */}
                        <div className="flex justify-between items-center p-2 border-b border-[#333] bg-[#111]">
                            <div className="flex items-center gap-2 text-[#4FD6BE] font-mono text-[10px]">
                                <Terminal size={12} />
                                <span className="tracking-widest">PRIVATE_MEMORY_BANK // -.- .-. .. ... - . -.</span>
                            </div>
                            <button onClick={() => setShowEasterEgg(false)} className="text-[#555] hover:text-[#FF6B35] transition-colors"><X size={16} /></button>
                        </div>

                        {/* Image */}
                        <div className="relative aspect-[4/3] w-full border-b border-[#333] group">
                            <img 
                                src={eggImage} 
                                alt="Memory" 
                                className="w-full h-full object-cover grayscale opacity-80 group-hover:grayscale-0 group-hover:opacity-100 transition-all duration-1000"
                            />
                            {/* CRT Scanline Effect */}
                            <div className="absolute inset-0 bg-[linear-gradient(rgba(18,16,16,0)_50%,rgba(0,0,0,0.25)_50%),linear-gradient(90deg,rgba(255,0,0,0.06),rgba(0,255,0,0.02),rgba(0,0,255,0.06))] z-10 bg-[length:100%_2px,3px_100%] pointer-events-none"></div>
                        </div>

                        {/* Text Area */}
                        <div className="p-8 text-center bg-[#0e0e10]">
                             <h2 className="text-xl md:text-2xl font-bold text-white font-futura tracking-widest uppercase mb-2 animate-pulse">
                                "Good night, Kristen"
                             </h2>
                             <div className="h-[1px] w-12 bg-[#FF6B35] mx-auto my-3"></div>
                             <p className="text-[10px] text-[#555] font-mono uppercase tracking-[0.2em]">	If, in a century or a millenium, our descendants walk among the stars, the masses will sing her praises.	</p>
                        </div>
                    </div>
                </div>
            </div>
        )}
    </>
  );
};

export default Sidebar;