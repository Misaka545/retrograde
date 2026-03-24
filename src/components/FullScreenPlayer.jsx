// src/components/FullScreenPlayer.jsx
import React from 'react';
import { ChevronDown, MoreHorizontal, Play, Pause, SkipBack, SkipForward, Shuffle, Repeat, Disc, X } from 'lucide-react';
import { usePlayer } from '../context/PlayerContext';
import { formatTime } from '../utils/timeUtils';

const FullScreenPlayer = ({ onClose }) => {
  const { 
    currentTrack, isPlaying, togglePlay, 
    handleNext, handlePrev, currentTime, setCurrentTime,
    isShuffle, setIsShuffle, repeatMode, setRepeatMode,
    playQueue, currentTrackIndex, audioRef,
    removeFromQueue 
  } = usePlayer();

  const handleSeek = (e) => {
      const newTime = parseFloat(e.target.value);
      setCurrentTime(newTime);
      if (audioRef.current) audioRef.current.currentTime = newTime;
  };

  const upcomingTracks = playQueue.slice(currentTrackIndex + 1);

  return (
    <div className="fixed inset-0 z-[60] bg-[#09090b] flex flex-col animate-in slide-in-from-bottom duration-300 font-sans text-white">
      
      {/* Background */}
      <div className="absolute inset-0 z-0 overflow-hidden pointer-events-none">
          <div className="absolute -top-[20%] -left-[20%] w-[70%] h-[70%] bg-[#FF6B35]/20 blur-[120px] rounded-full mix-blend-screen"></div>
          <div className="absolute top-[20%] right-[20%] w-[50%] h-[50%] bg-[#4FD6BE]/10 blur-[100px] rounded-full mix-blend-screen"></div>
          <div className="absolute inset-0 bg-black/40 backdrop-blur-3xl"></div>
      </div>

      {/* Header */}
      <div className="relative z-10 flex items-center justify-between px-6 py-6 md:px-12">
        <button onClick={onClose} className="p-2 hover:bg-white/10 rounded-full transition-colors"><ChevronDown size={32} strokeWidth={1.5} /></button>
        <div className="flex flex-col items-center">
            <span className="text-[10px] font-bold tracking-[0.2em] text-[#888] uppercase">Now Playing</span>
            <span className="text-xs font-bold tracking-widest uppercase text-white truncate max-w-[200px]">{currentTrack.album || "SINGLE TRACK"}</span>
        </div>
        <button className="p-2 hover:bg-white/10 rounded-full transition-colors"><MoreHorizontal size={24} /></button>
      </div>

      {/* MAIN CONTENT */}
      <div className="relative z-10 flex-1 flex flex-col lg:flex-row items-center justify-center gap-12 px-8 pb-12 overflow-y-auto custom-scrollbar">
        
        {/* LEFT: ARTWORK */}
        <div className="flex-1 flex justify-center lg:justify-end w-full max-w-xl">
            <div className="aspect-square w-full max-w-[400px] lg:max-w-[550px] relative group">
                <div className="absolute -inset-4 border border-white/5 rounded-sm pointer-events-none"></div>
                <div className="absolute top-0 left-0 w-6 h-6 border-t-2 border-l-2 border-[#FF6B35]"></div>
                <div className="absolute bottom-0 right-0 w-6 h-6 border-b-2 border-r-2 border-[#4FD6BE]"></div>
                <div className="w-full h-full shadow-[0_20px_60px_-10px_rgba(0,0,0,0.8)] bg-[#1a1a1a] relative overflow-hidden rounded-sm">
                    {currentTrack.coverArt ? <img src={currentTrack.coverArt} className="w-full h-full object-cover" /> : <div className="w-full h-full flex items-center justify-center bg-[#222]"><Disc size={96} className="text-[#333]" /></div>}
                </div>
            </div>
        </div>

        {/* RIGHT: CONTROLS & QUEUE LIST */}
        <div className="flex-1 w-full max-w-xl flex flex-col gap-8 lg:items-start items-center text-center lg:text-left h-full max-h-[600px]">
            
            {/* Controls Block */}
            <div className="w-full flex-shrink-0">
                <h1 className="text-3xl md:text-5xl font-bold text-white mb-2 leading-tight tracking-tight">{currentTrack.title}</h1>
                <p className="text-xl text-[#ccc] font-medium tracking-wide mb-6">{currentTrack.artist}</p>

                {/* Progress & Buttons */}
                <div className="w-full group mb-6">
                    <div className="relative h-2 w-full flex items-center cursor-pointer">
                        <div className="absolute top-1/2 -translate-y-1/2 w-full h-[4px] bg-[#333] rounded-full overflow-hidden">
                            <div className="h-full bg-white relative" style={{ width: `${(currentTime / (currentTrack.duration || 1)) * 100}%` }}></div>
                        </div>
                        <input type="range" min="0" max={currentTrack.duration || 0} step="0.1" value={currentTime} onChange={handleSeek} className="absolute inset-0 w-full h-full opacity-0 cursor-pointer z-20" />
                    </div>
                    <div className="flex justify-between text-xs font-mono text-[#888] mt-2">
                        <span>{formatTime(currentTime)}</span>
                        <span>{formatTime(currentTrack.duration)}</span>
                    </div>
                </div>

                <div className="flex items-center justify-center gap-8 w-full">
                    <button onClick={() => setIsShuffle(!isShuffle)} className={`transition-colors hover:scale-110 ${isShuffle ? 'text-[#FF6B35]' : 'text-[#666] hover:text-white'}`}><Shuffle size={24} /></button>
                    <button onClick={handlePrev} className="text-white hover:text-[#4FD6BE] transition-colors hover:scale-110"><SkipBack size={36} fill="currentColor" /></button>
                    <button onClick={togglePlay} className="w-20 h-20 bg-white text-black rounded-full flex items-center justify-center hover:scale-105 transition-transform shadow-[0_0_40px_rgba(255,255,255,0.15)] mx-2">
                        {isPlaying ? <Pause size={32} fill="black" /> : <Play size={32} fill="black" className="ml-1"/>}
                    </button>
                    <button onClick={handleNext} className="text-white hover:text-[#4FD6BE] transition-colors hover:scale-110"><SkipForward size={36} fill="currentColor" /></button>
                    <button onClick={() => setRepeatMode((prev) => (prev + 1) % 3)} className={`relative transition-colors hover:scale-110 ${repeatMode > 0 ? 'text-[#FF6B35]' : 'text-[#666] hover:text-white'}`}><Repeat size={24} />{repeatMode === 2 && <span className="absolute -top-1.5 right-[-2px] text-[9px] font-bold">1</span>}</button>
                </div>
            </div>

            {/* --- QUEUE LIST --- */}
            <div className="w-full flex-1 flex flex-col min-h-0 bg-[#1a1a1a]/40 backdrop-blur-md rounded-lg border border-white/5 overflow-hidden">
                <div className="flex justify-between items-center px-4 py-3 border-b border-white/5 bg-[#1a1a1a]/60">
                    <span className="text-xs font-bold text-white uppercase tracking-wider">Next in Queue</span>
                    <span className="text-[10px] text-[#555] font-mono">{upcomingTracks.length} TRACKS</span>
                </div>
                
                <div className="overflow-y-auto custom-scrollbar flex-1 p-2">
                    {upcomingTracks.length === 0 ? (
                        <div className="h-full flex items-center justify-center text-xs text-[#555] italic">End of queue</div>
                    ) : (
                        upcomingTracks.map((track, i) => {
                            const realIndex = currentTrackIndex + 1 + i;
                            
                            return (
                                <div key={i} className="flex items-center gap-3 p-2 hover:bg-white/5 rounded-md group transition-colors relative">
                                    {/* Số thứ tự */}
                                    <span className="text-[10px] text-[#444] font-mono w-4 text-center group-hover:hidden">{i + 1}</span>
                                    <div className="w-4 hidden group-hover:flex items-center justify-center">
                                        <Play size={10} fill="white" />
                                    </div>

                                    {/* Ảnh nhỏ */}
                                    <div className="w-8 h-8 bg-[#333] rounded overflow-hidden flex-shrink-0">
                                        {track.coverArt ? <img src={track.coverArt} className="w-full h-full object-cover"/> : <Disc size={12} className="text-[#555] m-auto"/>}
                                    </div>

                                    {/* Info */}
                                    <div className="min-w-0 flex-1 text-left">
                                        <div className="text-xs text-white truncate font-medium">{track.title}</div>
                                        <div className="text-[10px] text-[#666] truncate">{track.artist}</div>
                                    </div>

                                    {/* Nút Xóa */}
                                    <button 
                                        onClick={(e) => { e.stopPropagation(); removeFromQueue(realIndex); }}
                                        className="p-1.5 text-[#555] hover:text-[#FF6B35] hover:bg-[#333] rounded-full opacity-0 group-hover:opacity-100 transition-all"
                                        title="Xóa khỏi hàng đợi"
                                    >
                                        <X size={14} />
                                    </button>
                                </div>
                            );
                        })
                    )}
                </div>
            </div>

        </div>
      </div>
    </div>
  );
};

export default FullScreenPlayer;