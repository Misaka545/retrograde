// src/components/PlayerBar.jsx
import React, { useRef, useState, useEffect } from 'react';
import { Play, Pause, SkipBack, SkipForward, Volume2, Volume1, VolumeX, Disc, Heart, Shuffle, Repeat, ListMusic, Maximize2, Speaker } from 'lucide-react';
import { usePlayer } from '../context/PlayerContext';
import { formatTime } from '../utils/timeUtils';

const PlayerBar = ({ onOpenAlbum, onToggleFullScreen, onToggleQueue }) => {
    const {
        isPlaying, currentTrack, volume, setVolume, currentTime, setCurrentTime,
        togglePlay, handleNext, handlePrev, isShuffle, setIsShuffle, repeatMode, setRepeatMode,
        toggleLike, isLiked, audioRef, toggleMute, isMuted,
        audioDevices, selectedDeviceId, setAudioOutputDevice, getAudioDevices
    } = usePlayer();

    const [showDeviceMenu, setShowDeviceMenu] = useState(false);
    const deviceMenuRef = useRef(null);

    useEffect(() => {
        const handleClickOutside = (event) => {
            if (deviceMenuRef.current && !deviceMenuRef.current.contains(event.target)) {
                setShowDeviceMenu(false);
            }
        };
        document.addEventListener("mousedown", handleClickOutside);
        return () => document.removeEventListener("mousedown", handleClickOutside);
    }, []);

    const handleSeekChange = (e) => {
        const newTime = parseFloat(e.target.value);
        setCurrentTime(newTime);
        if (audioRef.current) audioRef.current.currentTime = newTime;
    };

    const handleVolumeChange = (e) => {
        setVolume(parseFloat(e.target.value));
    };

    const getDeviceName = (device) => {
        if (device.label) return device.label;
        return `Device ${device.deviceId.substring(0, 5)}...`;
    };

    const VolumeIcon = isMuted || volume === 0 ? VolumeX : volume < 0.5 ? Volume1 : Volume2;

    return (
        <div className="h-24 bg-[#0e0e10] border-t border-[#333] fixed bottom-0 left-0 right-0 z-50 flex items-center px-8 gap-10 shadow-[0_-5px_20px_rgba(0,0,0,0.5)]">

            {/* Background Grid */}
            <div className="absolute inset-0 opacity-10 pointer-events-none" style={{ backgroundImage: 'linear-gradient(90deg, #222 1px, transparent 1px)', backgroundSize: '20px 100%' }}></div>

            {/* PROGRESS BAR */}
            <div className="absolute -top-[6px] left-0 w-full h-[12px] group z-30 flex items-center cursor-pointer">
                <div className="absolute top-[4px] left-0 w-full h-[4px] bg-[#1a1a1a] pointer-events-none">
                    <div className="h-full bg-gradient-to-r from-[#FF6B35] via-[#E8C060] to-[#4FD6BE] relative" style={{ width: `${(currentTime / (currentTrack.duration || 1)) * 100}%` }}>
                        <div className="absolute right-[-4px] top-1/2 -translate-y-1/2 w-2 h-3 bg-[#4FD6BE] opacity-0 group-hover:opacity-100 transition-transform duration-100 scale-75 group-hover:scale-100"></div>
                    </div>
                </div>
                <input type="range" min="0" max={currentTrack.duration || 0} step="0.1" value={currentTime} onChange={handleSeekChange} className="w-full h-full opacity-0 cursor-pointer z-40" />
            </div>

            {/* LEFT: TRACK INFO */}
            <div className="flex items-center gap-5 w-1/4 min-w-[200px] z-10">
                <div
                    className="w-14 h-14 bg-[#1a1a1a] border border-[#444] flex items-center justify-center relative overflow-hidden flex-shrink-0 group cursor-pointer"
                    onClick={(e) => { e.stopPropagation(); onOpenAlbum(); }}
                    title="Đi tới Album"
                >
                    <div className="absolute inset-0 bg-[#FF6B35]/5 group-hover:bg-[#FF6B35]/20 transition-colors"></div>
                    {currentTrack.coverArt ? (
                        <img src={currentTrack.coverArt} className="w-full h-full object-cover p-[2px]" />
                    ) : (
                        <Disc size={28} className={`text-[#FF6B35] ${isPlaying ? 'animate-spin-slow' : ''}`} />
                    )}
                </div>
                <div className="flex flex-col overflow-hidden">
                    <div className="flex items-center gap-2 mb-1">
                        <span className="bg-[#4FD6BE] text-black text-[9px] font-bold px-1 tracking-wider rounded-[1px]">{isPlaying ? 'PLAYING' : 'READY'}</span>
                    </div>
                    <span
                        className="text-base font-bold tracking-wide uppercase text-white truncate hover:text-[#E8C060] transition-colors cursor-pointer"
                        onClick={(e) => { e.stopPropagation(); onOpenAlbum(); }}
                    >
                        {currentTrack.title || "NO_DATA"}
                    </span>
                    <span className="text-xs text-[#666] font-mono truncate tracking-wider">{currentTrack.artist || "UNKNOWN"}</span>
                </div>
            </div>

            {/* CENTER: CONTROLS */}
            <div className="flex-1 flex flex-col items-center justify-center gap-1 z-10">
                <div className="flex items-center gap-10">
                    <div className="flex items-center gap-4">
                        <button onClick={() => setIsShuffle(!isShuffle)} className={`w-8 h-8 flex items-center justify-center transition-colors ${isShuffle ? 'text-[#4FD6BE]' : 'text-[#444] hover:text-[#eee]'}`}><Shuffle size={16} /></button>
                        <SkipBack size={24} className="text-[#888] hover:text-white cursor-pointer transition-colors" onClick={handlePrev} />
                    </div>
                    <div onClick={togglePlay} className="w-16 h-14 bg-[#EAEAEA] text-black flex items-center justify-center hover:bg-white hover:shadow-[0_0_20px_rgba(255,255,255,0.4)] hover:scale-105 transition-all cursor-pointer relative" style={{ clipPath: 'polygon(25% 0%, 75% 0%, 100% 50%, 75% 100%, 25% 100%, 0% 50%)' }}>
                        {isPlaying ? <Pause size={28} fill="currentColor" className="z-10" /> : <Play size={28} fill="currentColor" className="ml-1 z-10" />}
                    </div>
                    <div className="flex items-center gap-4">
                        <SkipForward size={24} className="text-[#888] hover:text-white cursor-pointer transition-colors" onClick={handleNext} />
                        <button onClick={() => setRepeatMode((prev) => (prev + 1) % 3)} className={`w-8 h-8 flex items-center justify-center transition-colors relative ${repeatMode > 0 ? 'text-[#4FD6BE]' : 'text-[#444] hover:text-[#eee]'}`}>
                            <Repeat size={16} />
                            {repeatMode === 2 && <span className="absolute top-1 right-1 text-[8px] font-bold">1</span>}
                        </button>
                    </div>
                </div>
                <div className="flex justify-between w-full max-w-[200px] text-[9px] font-mono text-[#444]">
                    <span>{formatTime(currentTime)}</span>
                    <span>{formatTime(currentTrack.duration)}</span>
                </div>
            </div>

            {/* RIGHT: VOLUME & EXTRAS */}
            <div className="w-1/4 flex justify-end items-center gap-8 z-10 min-w-[250px]">

                {/* VOLUME SLIDER */}
                <div className="flex items-center gap-3 group relative w-24 h-8">
                    <button onClick={toggleMute} className="text-[#555] group-hover:text-white transition-colors focus:outline-none">
                        <VolumeIcon size={18} />
                    </button>
                    <div className="relative flex-1 h-full flex items-center">
                        <div className="flex items-end gap-[2px] h-6 w-full pointer-events-none">
                            {[...Array(15)].map((_, idx) => {
                                const barColor = idx < 5 ? '#4FD6BE' : idx < 10 ? '#E8C060' : '#FF6B35';
                                const isActive = (idx / 15) < volume && !isMuted;
                                return (
                                    <div key={idx} className="w-full bg-[#222] relative" style={{ height: '100%' }}>
                                        <div className="w-full absolute bottom-0 transition-all duration-75" style={{ height: isActive ? '100%' : '0%', backgroundColor: barColor, opacity: isActive ? 1 : 0 }}></div>
                                    </div>
                                )
                            })}
                        </div>
                        <input type="range" min="0" max="1" step="0.05" value={isMuted ? 0 : volume} onChange={handleVolumeChange} className="absolute inset-0 w-full h-full opacity-0 cursor-pointer z-20" />
                    </div>
                </div>

                {/* 2. EXTRAS GROUP */}
                <div className="flex items-center gap-5 text-[#555] border-l border-[#333] pl-6 relative h-8">

                    {/* Heart */}
                    <button onClick={() => toggleLike()} className={`transition-colors flex items-center justify-center ${isLiked ? 'text-[#FF6B35]' : 'hover:text-[#FF6B35]'}`}>
                        <Heart size={20} fill={isLiked ? "currentColor" : "none"} />
                    </button>

                    {/* Queue */}
                    <button onClick={onToggleQueue} title="Hàng đợi" className="hover:text-[#E8C060] transition-colors cursor-pointer focus:outline-none flex items-center justify-center">
                        <ListMusic size={20} />
                    </button>

                    {/* DEVICE SELECTOR */}
                    <div className="relative flex items-center justify-center" ref={deviceMenuRef}>
                        <button
                            onClick={() => {
                                setShowDeviceMenu(!showDeviceMenu);
                                if (!showDeviceMenu) getAudioDevices();
                            }}
                            className={`transition-colors flex items-center justify-center ${showDeviceMenu ? 'text-[#FF6B35]' : 'hover:text-[#4FD6BE]'}`}
                            title="Chọn thiết bị Output"
                        >
                            <Speaker size={20} />
                        </button>

                        {/* MENU POPUP */}
                        {showDeviceMenu && (
                            <div className="absolute bottom-full right-0 mb-12 w-72 bg-[#161616] border border-[#333] shadow-[0_0_30px_rgba(0,0,0,0.8)] rounded-sm p-1 z-[200] animate-in slide-in-from-bottom-2">
                                <div className="px-3 py-2 text-[9px] font-bold text-[#4FD6BE] border-b border-[#333] mb-1 font-mono tracking-wider uppercase flex justify-between">
                                    <span>SELECT OUTPUT</span>
                                    <span className="text-[#555]">///</span>
                                </div>
                                <div className="max-h-60 overflow-y-auto custom-scrollbar">
                                    {(!audioDevices || audioDevices.length === 0) && (
                                        <div className="px-3 py-2 text-xs text-[#666] italic">No devices found</div>
                                    )}
                                    {audioDevices && audioDevices.map(device => (
                                        <button
                                            key={device.deviceId}
                                            onClick={() => {
                                                setAudioOutputDevice(device.deviceId);
                                                setShowDeviceMenu(false);
                                            }}
                                            className={`w-full text-left px-3 py-3 text-xs flex items-center gap-3 hover:bg-[#222] transition-colors border-b border-[#222] last:border-0 ${selectedDeviceId === device.deviceId ? 'text-[#E8C060]' : 'text-[#ccc]'}`}
                                        >
                                            <div className={`w-2 h-2 rounded-full flex-shrink-0 ${selectedDeviceId === device.deviceId ? 'bg-[#E8C060] shadow-[0_0_5px_#E8C060]' : 'bg-[#333]'}`}></div>
                                            <span className="truncate font-medium">{getDeviceName(device)}</span>
                                        </button>
                                    ))}
                                </div>
                            </div>
                        )}
                    </div>

                    {/* Fullscreen */}
                    <button onClick={onToggleFullScreen} title="Toàn màn hình" className="hover:text-white transition-colors cursor-pointer flex items-center justify-center">
                        <Maximize2 size={18} />
                    </button>
                </div>
            </div>
        </div>
    );
};

export default PlayerBar;