use adapter_platform_windows::marker_oracle::{
    expected_pixel, verify, Phase, Rect, Rgba, HEIGHT, WIDTH,
};

fn paint(frame: &mut [u8], width: u32, rect: Rect, phase: Phase) {
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let offset = (((rect.y + y) * width + rect.x + x) * 4) as usize;
            frame[offset..offset + 4].copy_from_slice(&expected_pixel(phase, x, y));
        }
    }
}

fn raster(width: u32, height: u32) -> Vec<u8> {
    [22, 27, 34, 255].repeat((width * height) as usize)
}

fn crop(frame: &[u8], width: u32, rect: Rect) -> Vec<u8> {
    let mut result = Vec::new();
    for y in rect.y..rect.y + rect.height {
        let begin = ((y * width + rect.x) * 4) as usize;
        result.extend_from_slice(&frame[begin..begin + rect.width as usize * 4]);
    }
    result
}

#[test]
fn actual_pixels_locate_all_three_phases_without_uia_or_adapter_geometry() {
    for phase in [Phase::A, Phase::B, Phase::Updated] {
        let rect = Rect {
            x: 17,
            y: 13,
            width: WIDTH,
            height: HEIGHT,
        };
        let mut frame = raster(120, 80);
        paint(&mut frame, 120, rect, phase);
        let image = crop(&frame, 120, rect);
        assert_eq!(
            verify(
                Rgba::new(120, 80, &frame).unwrap(),
                Rgba::new(WIDTH, HEIGHT, &image).unwrap(),
                rect,
                phase
            ),
            Ok(rect)
        );
    }
}

#[test]
fn one_pixel_shift_cannot_pass_even_when_its_center_still_has_the_expected_color() {
    let rect = Rect {
        x: 17,
        y: 13,
        width: WIDTH,
        height: HEIGHT,
    };
    let mut frame = raster(120, 80);
    paint(&mut frame, 120, rect, Phase::A);
    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
        let shifted = Rect {
            x: (rect.x as i32 + dx) as u32,
            y: (rect.y as i32 + dy) as u32,
            ..rect
        };
        let image = crop(&frame, 120, shifted);
        let center = ((HEIGHT / 2 * WIDTH + WIDTH / 2) * 4) as usize;
        assert_eq!(&image[center..center + 4], &[30, 80, 220, 255]);
        assert!(verify(
            Rgba::new(120, 80, &frame).unwrap(),
            Rgba::new(WIDTH, HEIGHT, &image).unwrap(),
            shifted,
            Phase::A
        )
        .is_err());
        assert!(verify(
            Rgba::new(120, 80, &frame).unwrap(),
            Rgba::new(WIDTH, HEIGHT, &image).unwrap(),
            rect,
            Phase::A
        )
        .is_err());
    }
}

#[test]
fn duplicate_absent_and_wrong_phase_markers_are_rejected() {
    let rect = Rect {
        x: 17,
        y: 13,
        width: WIDTH,
        height: HEIGHT,
    };
    let mut frame = raster(180, 100);
    paint(&mut frame, 180, rect, Phase::A);
    let image = crop(&frame, 180, rect);
    assert!(verify(
        Rgba::new(180, 100, &frame).unwrap(),
        Rgba::new(WIDTH, HEIGHT, &image).unwrap(),
        rect,
        Phase::Updated
    )
    .is_err());
    paint(
        &mut frame,
        180,
        Rect {
            x: 100,
            y: 13,
            ..rect
        },
        Phase::A,
    );
    assert!(verify(
        Rgba::new(180, 100, &frame).unwrap(),
        Rgba::new(WIDTH, HEIGHT, &image).unwrap(),
        rect,
        Phase::A
    )
    .is_err());
    let absent = raster(180, 100);
    assert!(verify(
        Rgba::new(180, 100, &absent).unwrap(),
        Rgba::new(WIDTH, HEIGHT, &image).unwrap(),
        rect,
        Phase::A
    )
    .is_err());
}

#[test]
fn malformed_byte_lengths_and_wrong_dimensions_are_rejected() {
    assert!(Rgba::new(u32::MAX, u32::MAX, &[]).is_err());
    assert!(Rgba::new(0, 1, &[]).is_err());
    assert!(Rgba::new(1, 1, &[0; 3]).is_err());
    let raw = raster(64, 40);
    let small = raster(63, 40);
    assert!(verify(
        Rgba::new(64, 40, &raw).unwrap(),
        Rgba::new(63, 40, &small).unwrap(),
        Rect {
            x: 0,
            y: 0,
            width: 63,
            height: 40
        },
        Phase::A
    )
    .is_err());
}
