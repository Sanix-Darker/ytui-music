pub mod event;
mod utils;
use std::sync::Condvar;
use tui::{backend::CrosstermBackend, Terminal};
// These are the imports also used in __utils.rs__ so make this import shareable
mod shared_import {
    pub use fetcher;
    pub use libmpv;
    pub use serde::{Deserialize, Serialize};
    pub use std::convert::{From, Into, TryFrom, TryInto};
    pub use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    pub use tui::{
        backend::Backend,
        layout::{self, Alignment, Constraint, Direction, Layout, Rect},
        style::{self, Color, Modifier, Style},
        text::{self, Span, Spans, Text},
        widgets::{
            self, Block, BorderType, Borders, Cell, Gauge, List, ListItem, ListState, Paragraph,
            Row, Table, TableState, Tabs, Widget,
        },
    };
}
use crossterm::{
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use shared_import::*;

// Following several state defines the layout of the ui
// The ui is first splitted into 3 area arranged verticsally in order:
// --------------------------------------
// |            TopLayout               |
// -------------------------------------
// |                                    |
// |        MiddleLayout                |
// |                                    |
// --------------------------------------
// |        BottomLaout                 |
// --------------------------------------
// TopLayout and Bottom layout are of small height.
// 3 row for toplayout(2 for border and 1 for single line text) and
// 3 or 4 row(2 for border and 1 or 2 for music name and icons) for bottom layout
// bottomlayout. And all the remaining area is given to MiddleLayout
// TODO: Above specified division of height is not yet implemented. For now the dovision is based
// hardcoded in percentage. But instead change that to 3/4 row length for top/bottom layout
// and calculate the remaining row height. If remaining row height is less than like 6/7 row ask
// user to increase size of terminal window or zoom out/ decrease font size of terminal

// TopLayout is off full width in the top. This again contains two components which are splitted
// horizontally. These area are for searchbar (which covers more than half of layout) and
// helpbar (confusignly named as this shows the "Press ?" like message and also show the working
// status of app like "error occured", "no more result", "requesting data" and so on)
// They are splitted in such a ratio that searchbar covers much of area but helpbar should also get
// sufficient enouh so that any printed status is not hidden for now 85%:15% ratio
// ---------------------------------------
// |        Search Bar          | HelpBar |
// ---------------------------------------
pub struct TopLayout {
    layout: [Rect; 2],
}

// --------------------------------------
// |         |                          |
// |         |      MiddleLayout        |
// | SideBar |                          |
// |         |                          |
// --------------------------------------
// Sidebar holds the list of available quick navigation defined in struct `SidebarOption`
pub struct MainLayout {
    sidebar: SideBar,
    middle_section: MiddleLayout,
}

// TODO: Instead of having seperate struct to hold SideBar Rect define Rect directly in MainLayout
// So that this struct is removed and type of `MainLayout::sidebar` is Rect
pub struct SideBar {
    layout: Rect,
}

// --------------------------------------
// |                                    |
// |        Rect (musicbar)             |
// |                                    |
// |-------------------------------------
// |                                    |
// |        MiddleBottom                |
// |                                    |
// --------------------------------------
// Split the area vertically. first section is the area where musics are shown which is actually
// the individual video from youtube. See `Fetcher::MusicUnit` type
// See `MiddleBottom` for more
pub struct MiddleLayout {
    layout: Rect,
    bottom: MiddleBottom,
}

// ---------------------------------------
// |                  |                  |
// | Rect(playlisbar) | Rect (artistbar) |
// |                  |                  |
// ---------------------------------------
// splits the given area vertical half where left haf shows the list of playlist which is actually
// the playlist defined in youtube. See `Fetcher::PlaylistUnit` type
// right half show the list of artist which is actually
// the channel from youtube. See `Fetcher::ArtistUnit` type
pub struct MiddleBottom {
    layout: [Rect; 2],
}

// -------------------------------------
// |        Rect (MusicConroller)       |
// -------------------------------------
// TODO: Split this area horzontally where small portion in right half shows the info like
// suffle, repeat, pause/playing using icon
pub struct BottomLayout {
    layout: [Rect; 2],
}

// This is what final ui looks like
// ----------------------------------------------------------
// |    Searchbar                           |  Helpbar      |
// |--------------------------------------------------------|
// |         |                                              |
// |         |                                              |
// |         |                  MusicBar                    |
// |         |                                              |
// |         |                                              |
// | Sidebar |----------------------------------------------|
// |         |                      |                       |
// |         |                      |                       |
// |         |     PlaylistBar      |      ArtistBar        |
// |         |                      |                       |
// |         |                      |                       |
// |         |                      |                       |
// |--------------------------------------------------------|
// |                        BottomBar                       |
// ----------------------------------------------------------

// Sotres the position on which respective components (in which field is named after)
// are to be rendered
#[derive(Default)]
pub struct Position {
    pub search: Rect,
    pub help: Rect,
    pub shortcut: Rect,
    pub music: Rect,
    pub playlist: Rect,
    pub artist: Rect,
    pub music_info: Rect,
    pub bottom_icons: Rect,
}

// This function will:
// 1) Initilize the terminal backend
// 2) Get the layout of the ui
// 3) print content in ui
// 4) Run a loop waiting for state variable to change
// if user asks to quit the app(denoted by setting active window to None) -> Quit,
// else -> Update the ui
// Ui is always updated when notified. No checkes are done to weather the ui is really updated or
// not as algorithms defined in ternial backend is responsible for such checks.
// Ui is also updated in every REFRESH_RATE specified which will then sync the states like
// played duration to the ui. Also see documentation in __event.rs__ file
pub fn draw_ui(state: &mut Arc<Mutex<State>>, cvar: &mut Arc<Condvar>) {
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen).expect("Failed to enter alternate screen");
    terminal::enable_raw_mode().expect("Faild to enable raw mode");

    let backed = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backed).expect("Failed to create terminal from backend");

    terminal
        .clear()
        .unwrap_or_else(|_| eprintln!("Failed to clear the terminal"));
    terminal
        .hide_cursor()
        .unwrap_or_else(|_| eprintln!("Failed to hide cursor"));

    let mut previous_dimension: Rect = Rect::default();
    let mut position = Position::caclulate(&previous_dimension);
    let mut paint_ui = || {
        terminal
            .draw(|screen| {
                let mut state_unlocked = state.lock().unwrap();

                // As screen size doesn't change that often (is chaged when terminal window is
                // resized) so it is unnecessary to calcuate position for components in every draw
                // loop. Calculate once and recalculate when window size change
                let current_dimension = screen.size();
                if previous_dimension != current_dimension {
                    position = Position::caclulate(&current_dimension);
                    previous_dimension = current_dimension;
                }

                screen.render_widget(TopLayout::get_helpbox(&state_unlocked), position.help);
                screen.render_widget(TopLayout::get_searchbox(&state_unlocked), position.search);
                screen.render_stateful_widget(
                    SideBar::get_shortcuts(&state_unlocked),
                    position.shortcut,
                    &mut state_unlocked.sidebar,
                );

                // each of below three state keeps data as reference to prevent unnecessaru copy
                // i.e they holds immutable reference to internal field of state variable so state
                // is supposed to be borrowed immutable
                // Again, as render_stateful_widget takes state of widget as mutable reference
                // which is again inside our state variable so it becomes necessary to have two
                // reference (one immutable and one mutable) at once
                // One first thought is to wrap inside some cell but as this loop keeps running in
                // short time interval copying anything for that purpose would be consuming more
                // cpu. And it may be good time to play with unsafe
                let state_ptr = &mut state_unlocked as *mut std::sync::MutexGuard<'_, State<'_>>;
                let (mut music_state, mut playlist_state, mut artist_state);
                unsafe {
                    music_state = &mut (*state_ptr).musicbar.1;
                    playlist_state = &mut (*state_ptr).playlistbar.1;
                    artist_state = &mut (*state_ptr).artistbar.1;
                }

                let music_table = MiddleLayout::get_music_container(&mut state_unlocked);
                screen.render_stateful_widget(music_table, position.music, &mut music_state);
                let playlist_table = MiddleBottom::get_playlist_container(&mut state_unlocked);
                screen.render_stateful_widget(
                    playlist_table,
                    position.playlist,
                    &mut playlist_state,
                );
                let artist_table = MiddleBottom::get_artist_container(&mut state_unlocked);
                screen.render_stateful_widget(artist_table, position.artist, &mut artist_state);

                state_unlocked.refresh_mpv_status();

                screen.render_widget(
                    BottomLayout::get_status_bar(&state_unlocked),
                    position.music_info,
                );
                screen.render_widget(
                    BottomLayout::get_icons_set(&state_unlocked),
                    position.bottom_icons,
                );
            })
            .unwrap();
    };
    paint_ui();

    'reactor: loop {
        // Use if instead of match because if will drop the mutex while going to else branch
        // but match keeps locking the mutex until match expression finished
        if cvar.wait(state.lock().unwrap()).unwrap().active == Window::None {
            break 'reactor;
        } else {
            paint_ui();
        }
    }

    // Attempt to bring terminal in original state before thi appbut when any attempt is failed
    // do not panic but simply leave the message about failure and user will be responsibe to
    // handle their terminal on their own
    crossterm::terminal::disable_raw_mode().unwrap_or_else(|_| {
        eprintln!("Failed to leave raw mode. You may need to restart the terminal")
    });
    execute!(std::io::stdout(), LeaveAlternateScreen).unwrap_or_else(|_| {
        eprintln!("Failed to leave alternate mode. You may need to restart the terminal")
    });
    terminal
        .show_cursor()
        .unwrap_or_else(|_| eprintln!("Failed to show cursor. Try: stty sane"));
}

#[derive(Clone)]
pub enum SidebarOption {
    Trending = 0,
    YoutubeCommunity = 1,
    RecentlyPlayed = 2,
    Favourates = 3,
    Search = 4,
    None = 5,
}

#[derive(PartialEq, Clone)]
pub enum Window {
    Searchbar,
    Helpbar,
    Sidebar,
    Musicbar,
    Playlistbar,
    Artistbar,
    None,
}

pub struct BottomState {
    music_duration: Duration,
    music_elapse: Duration,
    // String inside the Some is the title of song being played
    // true in Some means that music is currently playing
    // false in Some means music is paused
    // None means playing nothing. eg: At the start of program
    playing: Option<(String, bool)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MusicbarSource {
    Search(String),
    Trending,
    YoutubeCommunity,
    RecentlyPlayed,
    Favourates,
    Playlist(String),
    Artist(String),
}
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PlaylistbarSource {
    Search(String),
    RecentlyPlayed,
    Favourates,
    Artist(String),
}
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ArtistbarSource {
    Search(String),
    RecentlyPlayed,
    Favourates,
}

pub struct State<'p> {
    pub help: &'p str,
    // First is state of the sidebar list itself
    // And second is the state that is actually active.
    // which remains same even selected() of ListState is changed.
    // second memeber of tuple is only changed when user press ENTER on given SidebarOption
    sidebar: ListState,
    pub musicbar: (Vec<fetcher::MusicUnit>, TableState),
    pub playlistbar: (Vec<fetcher::PlaylistUnit>, TableState),
    pub artistbar: (Vec<fetcher::ArtistUnit>, TableState),
    pub filled_source: (MusicbarSource, PlaylistbarSource, ArtistbarSource),
    bottom: BottomState,
    // First string is the actual string being typed on searchbar (to actually render)
    // If (musicbar or playlistbar or artistbar) is filled with search result
    // second memebr is Some(result_of_this_query) (to send to fetcher)
    // second member is the string of searchbar when use pressed ENTER last time in searchbar
    pub search: (String, String),
    pub active: Window,
    pub fetched_page: [Option<usize>; 3],
    player: libmpv::Mpv,
}
