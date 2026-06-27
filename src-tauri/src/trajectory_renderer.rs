// 概要: マウスジェスチャーの軌跡をネイティブWin32レイヤードウィンドウで描画
// 入出力:
//   - 入力: 軌跡座標の配列 Vec<(i32, i32)>
//   - 出力: 画面上に軌跡を描画（透明背景、最前面表示）
// 実装詳細:
//   - WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST でクリックスルー可能な透明ウィンドウ
//   - UpdateLayeredWindow + ARGB32ビットマップでアルファチャンネル制御
//   - メモリDC上でGDI描画し、背景α=0、軌跡α=255で明示的に管理
//   - 別スレッドでウィンドウメッセージループを実行

use std::sync::Mutex;
use std::time::Instant;

use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::Graphics::Gdi::*,
    Win32::UI::WindowsAndMessaging::*,
};

static RENDERER_HWND: Mutex<Option<isize>> = Mutex::new(None);
static TRAJECTORY_POINTS: Mutex<Vec<(i32, i32)>> = Mutex::new(Vec::new());
static IS_VISIBLE: Mutex<bool> = Mutex::new(false);
static WINDOW_OFFSET: Mutex<(i32, i32)> = Mutex::new((0, 0));
static WINDOW_SIZE: Mutex<(i32, i32)> = Mutex::new((0, 0));
static WINDOW_POS: Mutex<(i32, i32)> = Mutex::new((0, 0));
static MEMORY_DC: Mutex<Option<isize>> = Mutex::new(None);
static MEMORY_BITMAP: Mutex<Option<isize>> = Mutex::new(None);
static LAST_RENDER_TIME: Mutex<Option<Instant>> = Mutex::new(None);
static TRAJECTORY_BOUNDS: Mutex<Option<RECT>> = Mutex::new(None);
static PREVIOUS_BOUNDS: Mutex<Option<RECT>> = Mutex::new(None);

const WINDOW_CLASS_NAME: PCWSTR = w!("OpenMouseGestureTrajectory");
static LINE_COLOR_RGB: Mutex<u32> = Mutex::new(0x00228B22);
const LINE_WIDTH: i32 = 3;
const BOUNDS_MARGIN: i32 = LINE_WIDTH * 2 + 10;
const WM_UPDATE_TRAJECTORY: u32 = WM_USER + 1;
const MIN_FRAME_INTERVAL_MS: u64 = 16;

unsafe fn clear_memory_dc() {
    let mem_dc_opt = MEMORY_DC.lock().unwrap();
    if let Some(mem_dc_val) = *mem_dc_opt {
        let mem_dc = HDC(mem_dc_val as *mut _);
        let (width, height) = *WINDOW_SIZE.lock().unwrap();
        let _ = PatBlt(mem_dc, 0, 0, width, height, BLACKNESS);
    }
}

// 軌跡のバウンディングにウィンドウとDIBを合わせる（スナップショット版）
unsafe fn ensure_window_matches_bounds_with_snapshot(hwnd: HWND, snapshot_bounds: Option<RECT>) {
    if let Some(r) = snapshot_bounds {
        let left = r.left;
        let top = r.top;
        let w = (r.right - r.left).max(1);
        let h = (r.bottom - r.top).max(1);
        eprintln!("[ENSURE_SNAP] bounds=({},{})~({},{}) -> window pos=({},{}) size={}x{}",
            r.left, r.top, r.right, r.bottom, left, top, w, h);

        // 現在のウィンドウ情報
        let (cur_w, cur_h) = *WINDOW_SIZE.lock().unwrap();
        let (cur_x, cur_y) = *WINDOW_POS.lock().unwrap();

        let need_realloc = w != cur_w || h != cur_h;
        let need_move = left != cur_x || top != cur_y;

        if need_realloc {
            let mem_dc_val_opt = *MEMORY_DC.lock().unwrap();
            if let Some(mem_dc_val) = mem_dc_val_opt {
                let mem_dc = HDC(mem_dc_val as *mut _);

                // 新しいDIBを作成
                let bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: w,
                        biHeight: -h,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        biSizeImage: 0,
                        biXPelsPerMeter: 0,
                        biYPelsPerMeter: 0,
                        biClrUsed: 0,
                        biClrImportant: 0,
                    },
                    bmiColors: [RGBQUAD::default(); 1],
                };
                let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
                let new_dib = CreateDIBSection(
                    Some(mem_dc),
                    &bmi,
                    DIB_RGB_COLORS,
                    &mut bits,
                    None,
                    0,
                ).unwrap();

                // 新しいDIBを選択、古いものを破棄
                let old = SelectObject(mem_dc, new_dib.into());
                if !old.0.is_null() {
                    let _ = DeleteObject(old);
                }

                // 保存
                {
                    let mut mem_bmp = MEMORY_BITMAP.lock().unwrap();
                    *mem_bmp = Some(new_dib.0 as isize);
                }
                {
                    let mut size = WINDOW_SIZE.lock().unwrap();
                    *size = (w, h);
                }

                clear_memory_dc();
            }
        }

        if need_move || need_realloc {
            eprintln!("[ENSURE_SNAP] SetWindowPos: ({},{}) {}x{} (realloc={}, move={})",
                left, top, w, h, need_realloc, need_move);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                left,
                top,
                w,
                h,
                SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOREDRAW,
            );
            {
                let mut pos = WINDOW_POS.lock().unwrap();
                *pos = (left, top);
            }
            {
                let mut off = WINDOW_OFFSET.lock().unwrap();
                // ローカル座標変換用に、オフセット=ウィンドウ左上
                *off = (left, top);
            }
        }
    }
}

unsafe fn render_to_memory_dc_with_snapshot(snapshot_bounds: Option<RECT>, snapshot_points: Vec<(i32, i32)>) {
    let (offset_x, offset_y) = if let Some(rect) = snapshot_bounds {
        eprintln!("[RENDER_SNAP] bounds=({},{})~({},{}) -> offset=({},{})",
            rect.left, rect.top, rect.right, rect.bottom, rect.left, rect.top);
        (rect.left, rect.top)
    } else {
        eprintln!("[RENDER_SNAP] No bounds, using offset=(0,0)");
        (0, 0)
    };
    eprintln!("[RENDER_SNAP] will draw {} points with offset=({},{})", snapshot_points.len(), offset_x, offset_y);
    let (width, height) = *WINDOW_SIZE.lock().unwrap();
    
    let mem_dc_opt = MEMORY_DC.lock().unwrap();
    if let Some(mem_dc_val) = *mem_dc_opt {
        let mem_dc = HDC(mem_dc_val as *mut _);
        let _ = PatBlt(mem_dc, 0, 0, width, height, BLACKNESS);
        
        if snapshot_points.len() >= 2 {
            let line_color = *LINE_COLOR_RGB.lock().unwrap();
            let pen = CreatePen(PS_SOLID, LINE_WIDTH, COLORREF(line_color));
            let old_pen = SelectObject(mem_dc, pen.into());
            
            let _ = SetBkMode(mem_dc, TRANSPARENT);
            let _ = SetROP2(mem_dc, R2_COPYPEN);
            
            let _ = MoveToEx(mem_dc, snapshot_points[0].0 - offset_x, snapshot_points[0].1 - offset_y, None);
            for i in 1..snapshot_points.len() {
                let _ = LineTo(mem_dc, snapshot_points[i].0 - offset_x, snapshot_points[i].1 - offset_y);
            }
            
            SelectObject(mem_dc, old_pen);
            let _ = DeleteObject(pen.into());
            fix_alpha_channel(mem_dc, offset_x, offset_y, &snapshot_points);
        }
    }
}

unsafe fn fix_alpha_channel(mem_dc: HDC, offset_x: i32, offset_y: i32, snapshot_points: &[(i32, i32)]) {
    let bitmap = HBITMAP((*MEMORY_BITMAP.lock().unwrap()).unwrap() as *mut _);
    let (width, height) = *WINDOW_SIZE.lock().unwrap();
    
    if snapshot_points.is_empty() {
        return;
    }
    
    eprintln!("[FIX_ALPHA] offset=({},{}) {} points", offset_x, offset_y, snapshot_points.len());
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    
    for &(x, y) in snapshot_points.iter() {
        let local_x = x - offset_x;
        let local_y = y - offset_y;
        min_x = min_x.min(local_x - BOUNDS_MARGIN);
        min_y = min_y.min(local_y - BOUNDS_MARGIN);
        max_x = max_x.max(local_x + BOUNDS_MARGIN);
        max_y = max_y.max(local_y + BOUNDS_MARGIN);
    }
    
    min_x = min_x.max(0);
    min_y = min_y.max(0);
    max_x = max_x.min(width - 1);
    max_y = max_y.min(height - 1);
    
    if min_x >= max_x || min_y >= max_y {
        return;
    }
    
    let mut bits = vec![0u8; (width * height * 4) as usize];
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        bmiColors: [RGBQUAD::default(); 1],
    };
    
    let _ = GetDIBits(mem_dc, bitmap, 0, height as u32, Some(bits.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);
    
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let offset = ((y * width + x) * 4) as usize;
            let b = bits[offset];
            let g = bits[offset + 1];
            let r = bits[offset + 2];
            
            if r > 0 || g > 0 || b > 0 {
                bits[offset + 3] = 255;
            } else {
                bits[offset + 3] = 0;
            }
        }
    }
    
    let _ = SetDIBits(Some(mem_dc), bitmap, 0, height as u32, bits.as_ptr() as *const _, &bmi, DIB_RGB_COLORS);
}

unsafe fn update_layered_window_from_memory(hwnd: HWND) {
    let mem_dc_opt = MEMORY_DC.lock().unwrap();
    if let Some(mem_dc_val) = *mem_dc_opt {
        let mem_dc = HDC(mem_dc_val as *mut _);
        let (win_w, win_h) = *WINDOW_SIZE.lock().unwrap();
        let (win_x, win_y) = *WINDOW_POS.lock().unwrap();
        if win_w > 0 && win_h > 0 {
            let size = SIZE { cx: win_w, cy: win_h };
            let pt_src = POINT { x: 0, y: 0 };
            let pt_dst = POINT { x: win_x, y: win_y };
            eprintln!("[UPDATE_LW] dst=({},{}) size={}x{} src=(0,0)", win_x, win_y, win_w, win_h);
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let result = UpdateLayeredWindow(
                hwnd,
                None,
                Some(&pt_dst),
                Some(&size),
                Some(mem_dc),
                Some(&pt_src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            if result.is_err() {
                eprintln!("[TRAJECTORY_RENDERER] UpdateLayeredWindow failed: {:?}", result);
            }
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_UPDATE_TRAJECTORY => {
            let visible = wparam.0 != 0;
            if visible {
                // 現在のバウンディングと軌跡ポイントをスナップショット（処理中の更新を防ぐ）
                let (snapshot_bounds, snapshot_points) = {
                    let bounds = TRAJECTORY_BOUNDS.lock().unwrap();
                    let points = TRAJECTORY_POINTS.lock().unwrap();
                    (bounds.clone(), points.clone())
                };
                eprintln!("[WM_UPDATE] snapshot: bounds={:?}, points={}", snapshot_bounds, snapshot_points.len());
                
                ensure_window_matches_bounds_with_snapshot(hwnd, snapshot_bounds.clone());
                render_to_memory_dc_with_snapshot(snapshot_bounds, snapshot_points);
                update_layered_window_from_memory(hwnd);
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            } else {
                clear_memory_dc();
                update_layered_window_from_memory(hwnd);
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let mem_dc_opt = MEMORY_DC.lock().unwrap();
            let mem_bitmap_opt = MEMORY_BITMAP.lock().unwrap();
            
            if let Some(bitmap) = *mem_bitmap_opt {
                let _ = DeleteObject(HGDIOBJ(bitmap as *mut _));
            }
            if let Some(dc) = *mem_dc_opt {
                let _ = DeleteDC(HDC(dc as *mut _));
            }
            
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub fn init_renderer() -> Result<()> {
    std::thread::spawn(|| {
        unsafe {
            let hinstance = HINSTANCE::default();

            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(window_proc),
                hInstance: hinstance,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: HBRUSH::default(),
                lpszClassName: WINDOW_CLASS_NAME,
                ..Default::default()
            };

            RegisterClassExW(&wc);

            let _virtual_x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let _virtual_y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let _virtual_width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let _virtual_height = GetSystemMetrics(SM_CYVIRTUALSCREEN);

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                WINDOW_CLASS_NAME,
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(hinstance),
                None,
            ).unwrap_or_default();

            if hwnd == HWND::default() {
                eprintln!("[TRAJECTORY_RENDERER] ERROR: Failed to create window");
                return;
            }

            {
                let mut renderer_hwnd = RENDERER_HWND.lock().unwrap();
                *renderer_hwnd = Some(hwnd.0 as isize);
            }

            {
                let mut offset = WINDOW_OFFSET.lock().unwrap();
                *offset = (0, 0);
            }

            {
                let mut size = WINDOW_SIZE.lock().unwrap();
                *size = (1, 1);
            }

            {
                let mut pos = WINDOW_POS.lock().unwrap();
                *pos = (0, 0);
            }

            let screen_dc = GetDC(None);
            let mem_dc = CreateCompatibleDC(Some(screen_dc));

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: 1,
                    biHeight: -1,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                bmiColors: [RGBQUAD::default(); 1],
            };

            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(
                Some(mem_dc),
                &bmi,
                DIB_RGB_COLORS,
                &mut bits,
                None,
                0,
            ).unwrap();

            let _ = SelectObject(mem_dc, dib.into());
            let _ = ReleaseDC(None, screen_dc);

            {
                let mut memory_dc = MEMORY_DC.lock().unwrap();
                *memory_dc = Some(mem_dc.0 as isize);
            }

            {
                let mut memory_bitmap = MEMORY_BITMAP.lock().unwrap();
                *memory_bitmap = Some(dib.0 as isize);
            }

            clear_memory_dc();
            update_layered_window_from_memory(hwnd);
            let _ = ShowWindow(hwnd, SW_HIDE);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    });

    std::thread::sleep(std::time::Duration::from_millis(100));
    Ok(())
}

// 概要: 軌跡線の描画色を設定する
// 入出力:
//   - 入力: "#RRGGBB" 形式の文字列。不正な値は無視され既定色を維持する
//   - 出力: なし（以降の描画に反映）
// 実装詳細:
//   - GDIのCOLORREFはBGR順(0x00BBGGRR)のためRGBから並べ替える
//   - アルファ算出が黒背景(=透明)と線色を色で区別するため、純黒(#000000)は0x010101へ補正する
pub fn set_line_color_from_hex(hex: &str) {
    let trimmed = hex.trim().trim_start_matches('#');
    if trimmed.len() != 6 {
        return;
    }
    if let Ok(rgb) = u32::from_str_radix(trimmed, 16) {
        let r = (rgb >> 16) & 0xFF;
        let g = (rgb >> 8) & 0xFF;
        let b = rgb & 0xFF;
        let mut color_ref = (b << 16) | (g << 8) | r;
        if color_ref == 0 {
            color_ref = 0x010101;
        }
        *LINE_COLOR_RGB.lock().unwrap() = color_ref;
    }
}

pub fn update_trajectory(points: &[(i32, i32)], visible: bool) {
    let bounds_rect = if visible && !points.is_empty() {
        let mut min_x = i32::MAX;
        let mut max_x = i32::MIN;
        let mut min_y = i32::MAX;
        let mut max_y = i32::MIN;
        
        for &(x, y) in points {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        
        Some(RECT {
            left: min_x - BOUNDS_MARGIN,
            top: min_y - BOUNDS_MARGIN,
            right: max_x + BOUNDS_MARGIN,
            bottom: max_y + BOUNDS_MARGIN,
        })
    } else {
        None
    };
    
    {
        let mut trajectory = TRAJECTORY_POINTS.lock().unwrap();
        trajectory.clear();
        if visible {
            trajectory.extend_from_slice(points);
        }
    }

    {
        let mut is_visible = IS_VISIBLE.lock().unwrap();
        *is_visible = visible;
    }
    
    {
        let mut prev_bounds = PREVIOUS_BOUNDS.lock().unwrap();
        let mut bounds = TRAJECTORY_BOUNDS.lock().unwrap();
        
        if bounds_rect.is_some() {
            *prev_bounds = *bounds;
        } else if bounds.is_some() {
            *prev_bounds = *bounds;
        }
        *bounds = bounds_rect;
    }

    let should_render = {
        let mut last_time = LAST_RENDER_TIME.lock().unwrap();
        let now = Instant::now();
        let should_update = match *last_time {
            Some(last) => now.duration_since(last).as_millis() >= MIN_FRAME_INTERVAL_MS as u128,
            None => true,
        };
        if should_update {
            *last_time = Some(now);
        }
        should_update || !visible // 非表示要求は常に通す
    };

    if should_render {
        let hwnd = {
            let renderer_hwnd = RENDERER_HWND.lock().unwrap();
            renderer_hwnd.map(|h| HWND(h as *mut _))
        };

        if let Some(hwnd) = hwnd {
            // レンダラースレッドに通知（描画はwindow_procで集約）
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_UPDATE_TRAJECTORY, WPARAM(if visible {1} else {0}), LPARAM(0));
            }
        }
    }
}

pub fn append_trajectory_point(x: i32, y: i32) {
    {
        let mut trajectory = TRAJECTORY_POINTS.lock().unwrap();
        trajectory.push((x, y));
    }

    {
        let mut is_visible = IS_VISIBLE.lock().unwrap();
        *is_visible = true;
    }
    
    {
        let mut bounds = TRAJECTORY_BOUNDS.lock().unwrap();
        
        if let Some(ref mut rect) = *bounds {
            let old_bounds = rect.clone();
            rect.left = rect.left.min(x - BOUNDS_MARGIN);
            rect.top = rect.top.min(y - BOUNDS_MARGIN);
            rect.right = rect.right.max(x + BOUNDS_MARGIN);
            rect.bottom = rect.bottom.max(y + BOUNDS_MARGIN);
            eprintln!("[APPEND] point=({},{}) bounds: ({},{})~({},{}) -> ({},{})~({},{})",
                x, y, old_bounds.left, old_bounds.top, old_bounds.right, old_bounds.bottom,
                rect.left, rect.top, rect.right, rect.bottom);
        } else {
            *bounds = Some(RECT {
                left: x - BOUNDS_MARGIN,
                top: y - BOUNDS_MARGIN,
                right: x + BOUNDS_MARGIN,
                bottom: y + BOUNDS_MARGIN,
            });
            eprintln!("[APPEND] point=({},{}) initial bounds: ({},{})~({},{})",
                x, y, x - BOUNDS_MARGIN, y - BOUNDS_MARGIN, x + BOUNDS_MARGIN, y + BOUNDS_MARGIN);
        }
    }

    let mut last_time = LAST_RENDER_TIME.lock().unwrap();
    let now = Instant::now();
    
    let should_render = match *last_time {
        Some(last) => now.duration_since(last).as_millis() >= MIN_FRAME_INTERVAL_MS as u128,
        None => true,
    };
    
    if should_render {
        *last_time = Some(now);
        drop(last_time);

        let hwnd = {
            let renderer_hwnd = RENDERER_HWND.lock().unwrap();
            renderer_hwnd.map(|h| HWND(h as *mut _))
        };

        if let Some(hwnd) = hwnd {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_UPDATE_TRAJECTORY, WPARAM(1), LPARAM(0));
            }
        }
    }
}

pub fn clear_trajectory_display() {
    {
        let mut trajectory = TRAJECTORY_POINTS.lock().unwrap();
        trajectory.clear();
    }

    {
        let mut is_visible = IS_VISIBLE.lock().unwrap();
        *is_visible = false;
    }

    let hwnd = {
        let renderer_hwnd = RENDERER_HWND.lock().unwrap();
        renderer_hwnd.map(|h| HWND(h as *mut _))
    };

    if let Some(hwnd) = hwnd {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_UPDATE_TRAJECTORY, WPARAM(0), LPARAM(0));
        }
    }
}
