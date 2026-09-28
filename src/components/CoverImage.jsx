import React, { useState, useEffect, useRef, memo } from 'react';
import { Disc, ListMusic } from 'lucide-react';

const loadedUrlCache = new Set();

const CoverImage = memo(({ src, alt, className, type = 'album', isPlaying = false, size = 'md', highRes = false }) => {
    const alreadyCached = Boolean(src && loadedUrlCache.has(src));
    const [hasError, setHasError] = useState(false);
    const [isLoaded, setIsLoaded] = useState(alreadyCached);
    const [isVisible, setIsVisible] = useState(alreadyCached);
    const containerRef = useRef(null);

    useEffect(() => {
        const cached = Boolean(src && loadedUrlCache.has(src));
        setHasError(false);
        setIsLoaded(cached);
        setIsVisible(cached);
    }, [src]);

    useEffect(() => {
        if (!src || isVisible) return;

        const scrollParent = containerRef.current?.closest('.custom-scrollbar, .overflow-y-auto') || null;

        let observer;
        try {
            observer = new IntersectionObserver(
                (entries) => {
                    for (const entry of entries) {
                        if (entry.isIntersecting) {
                            setIsVisible(true);
                            if (observer) observer.disconnect();
                            break;
                        }
                    }
                },
                {
                    root: scrollParent,
                    rootMargin: '1200px 0px 1200px 0px'
                }
            );

            if (containerRef.current) {
                observer.observe(containerRef.current);
            }
        } catch {
            setIsVisible(true);
        }

        return () => {
            if (observer) observer.disconnect();
        };
    }, [src, isVisible]);

    const handleLoad = () => {
        setIsLoaded(true);
        if (src) loadedUrlCache.add(src);
    };

    const showFallback = !src || hasError;

    return (
        <div ref={containerRef} className={`relative bg-[#222] overflow-hidden ${className || ''}`}>
            {showFallback && (
                <div className="absolute inset-0 flex items-center justify-center">
                    {type === 'playlist' 
                        ? <ListMusic size={size === 'sm' ? 16 : 32} className="text-[#555]" />
                        : <Disc className={`text-[#333] ${size === 'sm' ? 'w-8 h-8' : 'w-full h-full p-8'} ${isPlaying ? 'animate-spin-slow' : ''}`} />
                    }
                </div>
            )}

            {!isLoaded && !showFallback && (
                <div className="absolute inset-0 bg-[#222] animate-pulse">
                    <div className="absolute inset-0 bg-gradient-to-r from-transparent via-[#333]/30 to-transparent animate-[shimmer_1.5s_infinite]" 
                         style={{ backgroundSize: '200% 100%' }} />
                </div>
            )}

            {isVisible && src && !hasError && (
                <img 
                    src={src}
                    alt={alt || ''}
                    decoding="async"
                    onLoad={handleLoad}
                    onError={() => setHasError(true)}
                    ref={(el) => {
                        if (el && el.complete && el.naturalWidth > 0 && !isLoaded) {
                            handleLoad();
                        }
                    }}
                    className={`w-full h-full object-cover transition-opacity duration-200 ${isLoaded ? 'opacity-100' : 'opacity-0'}`}
                />
            )}
        </div>
    );
});

export const prewarmCoverCache = () => {};
export const clearCoverCache = () => loadedUrlCache.clear();

export default CoverImage;
