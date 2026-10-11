// src/components/VirtualGrid.jsx
import React, { useState, useRef, useLayoutEffect, useEffect, useCallback } from 'react';

const GAP = 24;
const OVERSCAN_PX = 800;

const layoutCache = { width: 0, rowHeight: 0 };

// Mirrors the old Tailwind classes: grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5
const getColumnCount = () => {
    const w = typeof window !== 'undefined' ? window.innerWidth : 1280;
    if (w >= 1280) return 5;
    if (w >= 1024) return 4;
    if (w >= 768) return 3;
    return 2;
};

// Card = padding/border (34) + square cover (colWidth - 34) + mb-4 (16) + title (20) + mb-1 (4) + subtitle (15)
const estimateRowHeight = (width, columns) => {
    if (!width) return 300;
    const colWidth = (width - GAP * (columns - 1)) / columns;
    return Math.max(100, colWidth + 55);
};

const VirtualGrid = ({ items, scrollParentRef, renderItem }) => {
    const wrapperRef = useRef(null);
    const [width, setWidth] = useState(layoutCache.width);
    const [columns, setColumns] = useState(getColumnCount);
    const [measuredRowHeight, setMeasuredRowHeight] = useState(layoutCache.rowHeight);
    const [range, setRange] = useState({ start: 0, end: 0 });

    const rowHeight = measuredRowHeight || estimateRowHeight(width, columns);
    const rowStride = rowHeight + GAP;
    const rowCount = Math.ceil(items.length / columns);
    const totalHeight = rowCount > 0 ? rowCount * rowStride - GAP : 0;

    const computeRange = useCallback(() => {
        const wr = wrapperRef.current;
        if (!wr) return null;
        const sp = scrollParentRef?.current || wr.closest('.overflow-y-auto') || null;
        let viewTop;
        let viewHeight;
        if (sp) {
            viewTop = sp.getBoundingClientRect().top - wr.getBoundingClientRect().top;
            viewHeight = sp.clientHeight;
        } else {
            viewTop = -wr.getBoundingClientRect().top;
            viewHeight = window.innerHeight;
        }
        const start = Math.max(0, Math.floor((viewTop - OVERSCAN_PX) / rowStride));
        const end = Math.min(rowCount, Math.ceil((viewTop + viewHeight + OVERSCAN_PX) / rowStride));
        return { start, end: Math.max(start, end) };
    }, [scrollParentRef, rowStride, rowCount]);

    const applyRange = useCallback(() => {
        const next = computeRange();
        if (!next) return;
        setRange(prev => (prev.start === next.start && prev.end === next.end ? prev : next));
    }, [computeRange]);

    // Measure width / columns synchronously before first paint.
    useLayoutEffect(() => {
        const wr = wrapperRef.current;
        if (!wr) return;
        const w = wr.clientWidth;
        if (w && w !== width) setWidth(w);
        const c = getColumnCount();
        if (c !== columns) setColumns(c);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // Track container resizes (window resize, sidebar, etc.)
    useEffect(() => {
        const wr = wrapperRef.current;
        if (!wr || typeof ResizeObserver === 'undefined') return;
        const ro = new ResizeObserver(() => {
            const w = wr.clientWidth;
            setWidth(prev => (prev === w ? prev : w));
            const c = getColumnCount();
            setColumns(prev => (prev === c ? prev : c));
        });
        ro.observe(wr);
        return () => ro.disconnect();
    }, []);

    useEffect(() => {
        layoutCache.width = width;
    }, [width]);

    // Recompute the visible range whenever geometry changes.
    useLayoutEffect(() => {
        applyRange();
    }, [applyRange, items.length]);

    // Listen to scroll on the external scroll container with requestAnimationFrame throttle.
    useEffect(() => {
        const wr = wrapperRef.current;
        const target = scrollParentRef?.current || wr?.closest('.overflow-y-auto') || window;
        let rafId = null;
        const onScroll = () => {
            if (rafId !== null) return;
            rafId = requestAnimationFrame(() => {
                rafId = null;
                applyRange();
            });
        };
        target.addEventListener('scroll', onScroll, { passive: true });
        return () => {
            if (rafId !== null) cancelAnimationFrame(rafId);
            target.removeEventListener('scroll', onScroll);
        };
    }, [scrollParentRef, applyRange]);

    // Measure the real height of a rendered row.
    const rowObserverRef = useRef(null);
    const measureRowRef = useCallback((el) => {
        if (typeof ResizeObserver === 'undefined') return;
        if (!rowObserverRef.current) {
            rowObserverRef.current = new ResizeObserver((entries) => {
                const h = entries[0]?.target?.offsetHeight;
                if (h > 0) {
                    layoutCache.rowHeight = h;
                    setMeasuredRowHeight(prev => (Math.abs(prev - h) < 0.5 ? prev : h));
                }
            });
        }
        rowObserverRef.current.disconnect();
        if (el) rowObserverRef.current.observe(el);
    }, []);

    useEffect(() => () => rowObserverRef.current?.disconnect(), []);

    const rows = [];
    for (let r = range.start; r < Math.min(range.end, rowCount); r++) {
        const startIdx = r * columns;
        const rowItems = items.slice(startIdx, startIdx + columns);
        rows.push(
            <div
                key={r}
                ref={r === range.start ? measureRowRef : undefined}
                style={{
                    position: 'absolute',
                    top: r * rowStride,
                    left: 0,
                    right: 0,
                    display: 'grid',
                    gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
                    gap: GAP,
                }}
            >
                {rowItems.map((item, i) => renderItem(item, startIdx + i))}
            </div>
        );
    }

    return (
        <div ref={wrapperRef} style={{ position: 'relative', height: totalHeight }}>
            {rows}
        </div>
    );
};

export default VirtualGrid;
