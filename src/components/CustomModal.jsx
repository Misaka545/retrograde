import React from 'react';

const CustomModal = ({ isOpen, title, children, onConfirm, onCancel, confirmText = "OK", cancelText = "CANCEL", showCancel = true }) => {
  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/80 backdrop-blur-sm animate-in fade-in duration-200">
      <div className="bg-[#1a1a1a] w-full max-w-md border border-[#444] p-0 shadow-[0_0_30px_rgba(0,0,0,0.8)] relative">
        {/* Header */}
        <div className="bg-[#222] px-6 py-3 border-b border-[#333] flex items-center gap-2">
            <div className="w-2 h-2 bg-[#FF6B35]"></div>
            <h3 className="text-sm font-bold text-white tracking-widest font-mono uppercase">{title}</h3>
        </div>
        
        {/* Content */}
        <div className="p-6 text-[#ccc]">
          {children}
        </div>

        {/* Footer */}
        <div className="flex justify-end gap-3 p-4 bg-[#111] border-t border-[#333]">
          {showCancel && (
            <button onClick={onCancel} className="px-4 py-2 text-[10px] font-bold text-[#666] hover:text-white hover:bg-[#222] transition-colors border border-transparent hover:border-[#444] uppercase tracking-wider">
              {cancelText}
            </button>
          )}
          <button onClick={onConfirm} className="px-6 py-2 bg-[#E8C060] text-black text-[10px] font-bold hover:bg-white transition-colors uppercase tracking-wider">
            {confirmText}
          </button>
        </div>
        
        {/* Decorative Corner */}
        <div className="absolute bottom-0 right-0 w-3 h-3 border-b-2 border-r-2 border-[#FF6B35]"></div>
      </div>
    </div>
  );
};
export default CustomModal;