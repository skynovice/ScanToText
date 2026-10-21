# แปลงภาพเป็นข้อความ

โปรแกรม Native Windows ที่เขียนด้วย Rust สำหรับจับภาพจากกล้องหรือหน้าจอ เลือกหลายกรอบ (ROI) แล้วแปลงเป็นข้อความแบบ Batch ด้วย OCR.space หรือ Gemini พร้อม History, Capture Translate, Global Hotkey และ System Tray

ตัวโปรแกรมที่ใช้งานจริงเป็น Rust 100% ไม่ต้องติดตั้ง Python, PyQt หรือ OpenCV

## ใช้งาน

- เปิด `dist\ROI-OCR-Helper.exe` โปรแกรมจะซ่อนหน้าต่างทั้งหมดและแสดงเฉพาะไอคอนโปรแกรมแบบลอย
- คลิกไอคอนลอยโปร่งใสเพื่อเปิดเมนู Native Windows แบบเดียวกับ System Tray เมนูจะจัดด้านเปิดให้อยู่ในจออัตโนมัติ แล้วเลือก `แสดงหน้าต่างหลัก` หรือ `Welcome / วิธีใช้งาน`
- กล้อง: กด `เปิดกล้อง` ในแถบเครื่องมือก่อน จากนั้นคลิกภาพหรือกด `Space` เพื่อถ่าย แล้วลากกรอบข้อความ (เริ่มต้นกล้องปิดอยู่)
- จับหน้าจอ: กด `Alt+S` แล้วลาก Scope ของเอกสาร จากนั้นลาก Crop ได้หลายจุดและกด `Enter`
- OCR แล้ววางทันที: ชี้เมาส์ที่ช่องข้อความ กด `Alt+A` แล้วลากครอบบริเวณที่ต้องการ ระบบจะใช้ OCR engine ที่เลือกไว้ คลิกตำแหน่งเดิม และวางข้อความให้ (มี Clipboard สำรอง)
- OCR จากเมนู: เลือก `OCR to Clipboard (Alt+A)` จากไอคอนลอยหรือ System Tray แล้วลากครอบบริเวณที่ต้องการ ระบบจะคัดลอกผลลง Clipboard โดยไม่คลิกหรือวางอัตโนมัติ
- Capture Translate: กด `Alt+D` แล้วลากข้อความหนึ่งจุดเพื่ออ่านข้อความและแปลเป็นไทยด้วย Gemini ผลต้นฉบับและคำแปลจะแสดงเป็นกล่องข้อความข้างไอคอนลอย ค้างไว้จนกด `×`
- วางผลลัพธ์: กด `Alt+1` ถึง `Alt+9`
- ปิดหรือย่อหน้าต่างหลักเพื่อซ่อน โดยไอคอนลอยและ System Tray ยังทำงานต่อ
- ออกจากโปรแกรมจริงได้จาก `ออกจากโปรแกรม` บนไอคอนลอยหรือเมนู System Tray ซึ่งใช้ exit flow เดียวกัน

ตั้งค่า OCR engine, API key, Gemini model และขนาดตัวอักษรได้จาก Settings ข้อมูลตั้งค่าและ History อยู่ที่ `%APPDATA%\ROI-OCR-Helper` โดย API key ถูกเก็บเป็นข้อความธรรมดาในโปรไฟล์ Windows ของผู้ใช้

## พัฒนาและทดสอบ

ต้องใช้ Rust stable แบบ MSVC และ Windows SDK:

```powershell
.\test.ps1
```

ชุดทดสอบไม่เรียกกล้องหรือ network และตรวจ config, atomic file replacement, History/ROI state, image batch/rotation และตัว parser ของ OCR ทั้งสองระบบ นอกจากนี้ build ต้องผ่าน `clippy` โดยไม่มี warning

ทดสอบ Win32 virtual-desktop capture บนเครื่องที่มี interactive desktop ได้ด้วย:

```powershell
Set-Location .\rust-app
cargo test captures_virtual_desktop_pixels -- --ignored
```

Source code หลักอยู่ใน `rust-app\src`:

- `app.rs` — GUI, workflow, tray และ global hotkey
- `camera.rs` — กล้องผ่าน Windows Media Foundation
- `platform.rs` — screen capture, clipboard/paste, DPI และ single instance ผ่าน Win32
- `ocr.rs` — OCR.space และ Gemini
- `history.rs`, `image_ops.rs`, `config.rs` — persistence และ image pipeline

## Build

```powershell
.\build.ps1
```

ผลลัพธ์คือ `dist\ROI-OCR-Helper.exe` แบบ native single executable พร้อม icon และ version metadata ไม่มี Python runtime รวมอยู่ภายใน

ไอคอนคำสั่งใน Toolbar, เมนูปุ่มลอย และ System Tray มาจาก Google Material Design Icons ภายใต้ Apache License 2.0 โดยมีรายละเอียดและสำเนาไลเซนส์อยู่ใน `assets\icons`

