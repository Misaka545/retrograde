// src/components/QueuePopup.jsx
import React, { useEffect, useRef } from 'react';
import { X, Play, Trash2 } from 'lucide-react';
import { usePlayer } from '../context/PlayerContext';
import { formatTime } from '../utils/timeUtils';

const QueuePopup = ({ onClose }) => {
  const { playQueue, currentTrackIndex, startAlbumPlayback, removeFromQueue } = usePlayer();
  const popupRef = useRef(null);

  useEffect(() => {
    const handleClickOutside = (e) => {
      if (popupRef.current && !popupRef.current.contains(e.target)) {
        onClose();
      }
    };
    setTimeout(() => document.addEventListener('click', handleClickOutside), 100);
    return () => document.removeEventListener('click', handleClickOutside);
  }, []);

  return (
    <div 
        ref={popupRef}
        className="fixed bottom-28 right-8 w-80 md:w-96 bg-[#0e0e10]/95 backdrop-blur-xl border border-[#333] shadow-[0_0_30px_rgba(0,0,0,0.8)] z-40 rounded-sm flex flex-col animate-in slide-in-from-bottom-4 duration-200 overflow-hidden"
        style={{ maxHeight: 'calc(100vh - 150px)' }}
    >
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-[#333] bg-[#161616]">
            <div className="flex items-center gap-2">
                <span className="w-2 h-2 bg-[#E8C060] animate-pulse"></span>
                <span className="text-xs font-bold tracking-widest text-[#ccc] uppercase font-mono">Queue System</span>
            </div>
            <button onClick={onClose} className="text-[#555] hover:text-white transition-colors">
                <X size={16} />
            </button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto custom-scrollbar p-0">
            {playQueue.length === 0 ? (
                <div className="p-8 text-center text-[#555] font-mono text-xs italic">
                    QUEUE_EMPTY
                </div>
            ) : (
                <div>
                    {/* Bài đang phát */}
                    <div className="px-4 py-2 text-[10px] text-[#4FD6BE] font-bold tracking-wider uppercase bg-[#4FD6BE]/5 border-b border-[#333]">
                        Current Process
                    </div>
                    {playQueue[currentTrackIndex] && (
                        <div className="flex items-center gap-3 p-3 bg-[#ffffff]/5 border-l-2 border-[#4FD6BE]">
                            <div className="relative w-8 h-8 bg-[#222]">
                                <img src={playQueue[currentTrackIndex].coverArt} className="w-full h-full object-cover" />
                                <div className="absolute inset-0 flex items-center justify-center bg-black/40">
                                    <img src="https://open.spotifycdn.com/cdn/images/equaliser-animated-green.f93a2bf4.gif" className="w-3 h-3 grayscale opacity-80" />
                                </div>
                            </div>
                            <div className="min-w-0 flex-1">
                                <div className="text-sm font-bold text-[#4FD6BE] truncate">{playQueue[currentTrackIndex].title}</div>
                                <div className="text-[10px] text-[#888] truncate uppercase">{playQueue[currentTrackIndex].artist}</div>
                            </div>
                        </div>
                    )}

                    {/* Các bài tiếp theo */}
                    <div className="px-4 py-2 text-[10px] text-[#E8C060] font-bold tracking-wider uppercase border-b border-[#333] mt-2">
                        Next Sequence
                    </div>
                    
                    {playQueue.map((track, i) => {
                        if (i <= currentTrackIndex) return null; // Ẩn bài đã/đang phát
                        return (
                            <div key={i} className="group flex items-center gap-3 p-3 hover:bg-[#ffffff]/5 border-l-2 border-transparent hover:border-[#E8C060] transition-all cursor-pointer relative">
                                <div className="text-[10px] text-[#444] font-mono w-4 text-center">{i + 1}</div>
                                <div className="min-w-0 flex-1" onClick={() => startAlbumPlayback(playQueue, i)}>
                                    <div className="text-sm font-medium text-[#ccc] group-hover:text-white truncate transition-colors">{track.title}</div>
                                    <div className="text-[10px] text-[#555] truncate uppercase">{track.artist}</div>
                                </div>
                                <button 
                                    onClick={(e) => { e.stopPropagation(); removeFromQueue(i); }}
                                    className="p-1.5 text-[#555] hover:text-[#FF6B35] opacity-0 group-hover:opacity-100 transition-all"
                                >
                                    <Trash2 size={14} />
                                </button>
                            </div>
                        )
                    })}
                    
                    {currentTrackIndex >= playQueue.length - 1 && (
                        <div className="p-4 text-center text-[#444] font-mono text-[10px]">END_OF_QUEUE</div>
                    )}
                </div>
            )}
        </div>
    </div>
  );
};

export default QueuePopup;