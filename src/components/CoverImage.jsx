// src/components/CoverImage.jsx
import React, { useState, useEffect, useRef, memo } from 'react';
import { Disc, ListMusic } from 'lucide-react';

const blobCache = new Map();
const failedUrls = new Set();

const loadImageAsBlob = async (fileUrl) => {
    if (!fileUrl) return null;
    if (failedUrls.has(fileUrl)) return null;
    if (blobCache.has(fileUrl)) return blobCache.get(fileUrl);

    try {
        const response = await fetch(fileUrl);
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        const blob = await response.blob();
        const blobUrl = URL.createObjectURL(blob);
        blobCache.set(fileUrl, blobUrl);
        return blobUrl;
    } catch (err) {
        failedUrls.delete(fileUrl);
        return null;
    }
};

const CoverImage = memo(({ src, alt, className, type = 'album', isPlaying = false, size = 'md' }) => {
    const [blobSrc, setBlobSrc] = useState(() => blobCache.get(src) || null);
    const [hasError, setHasError] = useState(false);
    const [isLoaded, setIsLoaded] = useState(!!blobCache.get(src));
    const mountedRef = useRef(true);

    useEffect(() => {
        mountedRef.current = true;
        if (!src) return;

        if (blobCache.has(src)) {
            setBlobSrc(blobCache.get(src));
            setIsLoaded(true);
            setHasError(false);
            return;
        }

        let cancelled = false;
        loadImageAsBlob(src).then(url => {
            if (cancelled || !mountedRef.current) return;
            if (url) {
                setBlobSrc(url);
                setIsLoaded(true);
            } else {
                setHasError(true);
            }
        });

        return () => { cancelled = true; };
    }, [src]);

    useEffect(() => {
        return () => { mountedRef.current = false; };
    }, []);

    if (!src || hasError) {
        return (
            <div className={`flex items-center justify-center bg-[#222] ${className || ''}`}>
                {type === 'playlist' 
                    ? <ListMusic size={size === 'sm' ? 16 : 32} className="text-[#555]" />
                    : <Disc className={`text-[#333] ${size === 'sm' ? 'w-8 h-8' : 'w-full h-full p-8'} ${isPlaying ? 'animate-spin-slow' : ''}`} />
                }
            </div>
        );
    }

    return (
        <div className={`relative bg-[#222] ${className || ''}`}>
            {!isLoaded && (
                <div className="absolute inset-0 bg-[#222] animate-pulse">
                    <div className="absolute inset-0 bg-gradient-to-r from-transparent via-[#333]/30 to-transparent animate-[shimmer_1.5s_infinite]" 
                         style={{ backgroundSize: '200% 100%' }} />
                </div>
            )}
            <img 
                src={blobSrc || src}
                alt={alt || ''}
                loading="lazy"
                decoding="async"
                onLoad={() => setIsLoaded(true)}
                onError={() => setHasError(true)}
                className={`w-full h-full object-cover transition-opacity duration-300 ${isLoaded ? 'opacity-100' : 'opacity-0'}`}
            />
        </div>
    );
});

export const prewarmCoverCache = (urls) => {
    const uniqueUrls = [...new Set(urls)].filter(u => u && !blobCache.has(u));
    
    const BATCH = 4;
    let idx = 0;
    
    const loadBatch = () => {
        const batch = uniqueUrls.slice(idx, idx + BATCH);
        if (batch.length === 0) return;
        idx += BATCH;
        
        Promise.all(batch.map(loadImageAsBlob)).then(() => {
            if (idx < uniqueUrls.length) {
                requestIdleCallback ? requestIdleCallback(loadBatch) : setTimeout(loadBatch, 50);
            }
        });
    };
    
    if (typeof requestIdleCallback !== 'undefined') {
        requestIdleCallback(loadBatch);
    } else {
        setTimeout(loadBatch, 100);
    }
};

export const clearCoverCache = () => {
    blobCache.forEach(blobUrl => URL.revokeObjectURL(blobUrl));
    blobCache.clear();
    failedUrls.clear();
};

export default CoverImage;
