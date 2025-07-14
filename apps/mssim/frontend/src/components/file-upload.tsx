/* eslint-disable @typescript-eslint/no-explicit-any */
"use client";

import type React from "react";

import { Upload } from "lucide-react";
import { useRef } from "react";

interface FileUploadProps {
  onFileLoad: (data: any) => void;
}

export default function FileUpload({ onFileLoad }: FileUploadProps) {
  const fileInputRef = useRef<HTMLInputElement>(null);

  const handleFileUpload = (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (file) {
      const reader = new FileReader();
      reader.onload = (e) => {
        try {
          const jsonData = JSON.parse(e.target?.result as string);
          onFileLoad(jsonData);
        } catch (error) {
          console.warn(error);
          alert("Invalid JSON file");
        }
      };
      reader.readAsText(file);
    }
  };

  return (
    <>
      <input
        ref={fileInputRef}
        type="file"
        accept=".json"
        onChange={handleFileUpload}
        className="hidden"
      />
      <button
        onClick={() => fileInputRef.current?.click()}
        className="flex items-center gap-2 px-4 py-2 rounded-lg text-white hover:bg-white hover:text-[#32006D] transition-colors"
        style={{ backgroundColor: "#B7A57A" }}
      >
        <Upload size={20} />
        Upload JSON
      </button>
    </>
  );
}
