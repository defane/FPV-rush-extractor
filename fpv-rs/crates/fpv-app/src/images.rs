//! Decode posters off the UI thread and bound the GPU texture cache.
use eframe::egui::{self, TextureHandle};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};

pub struct Images {
    textures: HashMap<String, TextureHandle>,
    pending: HashSet<String>,
    failed: HashSet<String>,
    order: VecDeque<String>,
    sender: Sender<String>,
    results: Receiver<(String, Result<egui::ColorImage, String>)>,
}
impl Images {
    pub fn new(context: egui::Context) -> Self {
        let (sender, requests) = mpsc::channel::<String>();
        let (done, results) = mpsc::channel();
        std::thread::spawn(move || {
            for path in requests {
                let result = image::open(&path)
                    .map(|image| {
                        let image = image.thumbnail(1280, 720).to_rgba8();
                        egui::ColorImage::from_rgba_unmultiplied(
                            [image.width() as usize, image.height() as usize],
                            image.as_raw(),
                        )
                    })
                    .map_err(|error| error.to_string());
                if done.send((path, result)).is_err() {
                    break;
                }
                context.request_repaint();
            }
        });
        Self {
            textures: HashMap::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            order: VecDeque::new(),
            sender,
            results,
        }
    }
    pub fn get(&mut self, context: &egui::Context, path: &str) -> Option<TextureHandle> {
        while let Ok((path, result)) = self.results.try_recv() {
            self.pending.remove(&path);
            if let Ok(image) = result {
                self.textures.insert(
                    path.clone(),
                    context.load_texture(&path, image, egui::TextureOptions::LINEAR),
                );
                self.order.push_back(path);
                while self.order.len() > 128 {
                    if let Some(old) = self.order.pop_front() {
                        self.textures.remove(&old);
                        self.pending.remove(&old);
                    }
                }
            } else {
                self.failed.insert(path);
            }
        }
        if let Some(texture) = self.textures.get(path) {
            return Some(texture.clone());
        }
        if !self.failed.contains(path)
            && self.pending.len() < 32
            && self.pending.insert(path.into())
        {
            let _ = self.sender.send(path.into());
        }
        None
    }

    pub fn failed(&self, path: &str) -> bool {
        self.failed.contains(path)
    }
}
