# แปลงภาพเป็นข้อความ (ROI OCR Helper)

โปรแกรม Native Windows สำหรับจับภาพจากกล้องหรือหน้าจอ เลือกกรอบข้อความ (ROI) หลายจุด แล้วแปลงเป็นข้อความด้วย OCR.Space หรือ Google Gemini พร้อมเครื่องมือแปลภาษา, ประวัติภาพ, Global Hotkey, ไอคอนลอย และ System Tray

เวอร์ชันปัจจุบัน: **1.4.4** · เขียนด้วย Rust 100% ไม่ต้องติดตั้ง Python, PyQt หรือ OpenCV

## เริ่มใช้งาน

1. เปิดโปรแกรม Welcome จะแสดงทุกครั้งที่เริ่มโปรแกรม โดยหัวข้อคู่มือทั้ง 10 ข้อจะยุบไว้ เลือกขยายเฉพาะหัวข้อที่ต้องการ
2. เปิด **Settings** แล้วตั้งค่า API Key ของ OCR.Space หรือ Google Gemini ก่อนเริ่มแปลงภาพ
3. หน้าต่างหลักเริ่มซ่อนอยู่ กล้องปิดไว้ และไอคอนโปรแกรมแบบลอยกับ System Tray ใช้เปิดเมนูหรือหน้าต่างหลักได้

## ฟังก์ชันและคีย์ลัด

- **กล้อง:** กด `เปิดกล้อง` แล้วเลือกอุปกรณ์ หากมีกล้องหลายตัว คลิกภาพสดหรือกด `Space` เพื่อถ่ายภาพ
- **จับภาพหน้าจอ — `Alt+S`:** ลากกำหนดพื้นที่ภาพ (Scope) แล้วลากครอบข้อความเป็น Crop ได้หลายกรอบ กด `Enter` เพื่อจบและเริ่ม OCR; ชี้เมาส์บนกรอบแล้วกด `×` เพื่อลบเฉพาะกรอบนั้น หรือกดคลิกขวา/`Esc` เพื่อยกเลิก
- **OCR แล้ววางทันที — `Alt+A`:** วางเคอร์เซอร์ไว้ในช่องข้อความปลายทางก่อน กด `Alt+A` แล้วลากครอบบริเวณที่ต้องการ โปรแกรมจะ OCR ด้วย Engine ที่เลือก คลิกตำแหน่งเดิม และวางผลลัพธ์ให้ พร้อมเก็บข้อความไว้ใน Clipboard
- **OCR ลง Clipboard:** เลือก `OCR to Clipboard (Alt+A)` จากเมนูไอคอนลอยหรือ System Tray แล้วลากครอบบริเวณที่ต้องการ ผลลัพธ์จะถูกคัดลอกไว้โดยไม่คลิกหรือวางอัตโนมัติ
- **แก้ข้อความที่พิมพ์ผิดภาษา — `Alt+C`:** เลือกข้อความที่พิมพ์ด้วยผังแป้นพิมพ์ผิด แล้วกด `Alt+C` เพื่อแทนที่ข้อความด้วยการสลับระหว่าง US QWERTY และ Thai Kedmanee เช่น `l;ylfu8iy[` → `สวัสดีครับ` ฟังก์ชันนี้ทำงานกับข้อความที่เลือก ไม่ได้ใช้ OCR หรือ AI
- **จับข้อความและแปล — `Alt+D`:** ลากเลือกบริเวณข้อความ โปรแกรมใช้ Gemini แปลเป็นไทยและแสดงต้นฉบับกับคำแปลในหน้าต่างข้างไอคอนลอย หน้าต่างค้างจนกด `×`; ขณะเปิดผลแปล `Alt+1` คัดลอกต้นฉบับ และ `Alt+2` คัดลอกคำแปล
- **หน้าต่างหลักและ ROI:** เปิดภาพจากกล้อง, จับภาพหน้าจอ หรือ History แล้วลากเพิ่มกรอบข้อความได้หลายกรอบ แก้ไขหรือคัดลอกผลแต่ละกรอบได้ และเลือก OCR.Space (เร็ว) หรือ Gemini (แม่นยำ)
- **วางผลลัพธ์ — `Alt+1` ถึง `Alt+9`:** วางข้อความจาก ROI ลำดับที่ตรงกันลงในโปรแกรมที่กำลังใช้งาน (ยกเว้นเมื่อหน้าต่าง Capture Translate เปิดอยู่ ซึ่ง `Alt+1`/`Alt+2` ใช้คัดลอกข้อความแปล)
- **History:** เปิดดูภาพเดิมและผล OCR เพื่อทำงานต่อได้; เลือกหลายภาพและลบผ่านหน้าต่างยืนยัน
- **ไอคอนลอยและ System Tray:** ใช้เปิดเมนู, Welcome, หน้าต่างหลัก, จับภาพ, OCR to Clipboard, แปล, History, Settings หรือออกจากโปรแกรม การปิดหน้าต่างหลักเป็นการซ่อนโปรแกรม ไม่ใช่ออกจากโปรแกรม

## การตั้งค่าและข้อมูลผู้ใช้

เลือก OCR engine, ใส่ API Key, ตั้งค่าข้อความ และจัดการกล้องได้จากหน้าต่าง **Settings** ซึ่งแยกจากหน้าต่างหลัก ข้อมูลตั้งค่าและ History อยู่ที่:

```text
%APPDATA%\ROI-OCR-Helper
```

API Key ถูกเก็บเป็นข้อความธรรมดาในโปรไฟล์ Windows ของผู้ใช้ ควรใช้บัญชี Windows ที่เชื่อถือได้และอย่าแชร์ไฟล์ตั้งค่าที่มี Key อยู่

## Build บน Windows

ต้องติดตั้ง Rust stable ผ่าน rustup และ Visual Studio Build Tools พร้อม workload **Desktop development with C++** จากนั้นเปิด PowerShell ที่โฟลเดอร์โปรเจกต์

Build อย่างเดียวโดยไม่รัน Test หรือ Clippy:

```powershell
cargo build --manifest-path ".\rust-app\Cargo.toml" --release --locked
New-Item -ItemType Directory -Force -Path ".\dist" | Out-Null
Copy-Item ".\rust-app\target\release\roi-ocr-helper.exe" ".\dist\ROI-OCR-Helper.exe" -Force
```

ไฟล์พร้อมใช้อยู่ที่ `dist\ROI-OCR-Helper.exe` ส่วน `.\build.ps1` จะรัน `test.ps1` ก่อน Build; หากต้องการ Build อย่างเดียวให้ใช้คำสั่งด้านบน ดูรายละเอียดเพิ่มเติมใน [HowToBuild.txt](HowToBuild.txt)

หากต้องการรันชุดตรวจสอบแยกต่างหาก ใช้ `.\test.ps1` ซึ่งทำ `cargo fmt --check`, `cargo test` และ `cargo clippy` โดยคำสั่งเหล่านี้ไม่ได้ถูกรันระหว่างการ Build แบบตรงด้านบน

## โครงสร้างซอร์สโค้ด

- `rust-app/src/app.rs` — UI, workflows, System Tray และ hotkeys
- `rust-app/src/camera.rs` — กล้องผ่าน Windows Media Foundation
- `rust-app/src/platform.rs` — screen capture, clipboard/paste, DPI และ Win32 integration
- `rust-app/src/ocr.rs` — OCR.Space และ Gemini
- `rust-app/src/history.rs`, `image_ops.rs`, `config.rs` — ประวัติภาพ, image processing และการตั้งค่า

ไอคอนคำสั่งมาจาก Google Material Design Icons ภายใต้ Apache License 2.0; รายละเอียดและสำเนาไลเซนส์อยู่ใน `assets/icons`
