use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::{sync::atomic::Ordering, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const PORT: u16 = 2000;
const LISTEN_ADDRESS: &str = "0.0.0.0";
const EXISTANCE_BROADCAST_MILLIS: u64 = 1500;
type StdinMsg = [u8; 32];

const LISTEN_SOCKET: (&str, u16) = (LISTEN_ADDRESS, PORT);

static IN_CONNECTION: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut name: StdinMsg = [0; 32];
    name[0..5].copy_from_slice(b"kalle");
    tokio::spawn(broadcast_existance(name));
    let peers = Arc::new(Mutex::new(HashMap::new()));
    tokio::spawn(listen_for_peers(peers.clone(), name));
    tokio::spawn(print_available_peers(peers.clone()));

    let (stdin_watch_sender, stdin_watch_receiver) = tokio::sync::watch::channel([0; 32]);

    tokio::spawn(async move {
        stdin_listener(stdin_watch_sender).await.unwrap();
    });
    let stdin_watch_receiver_clone = stdin_watch_receiver.clone();
    tokio::spawn(async move {
        tcp_connection_listener(stdin_watch_receiver_clone)
            .await
            .unwrap();
    });
    tokio::spawn(async move {
        tcp_connection_creator(stdin_watch_receiver, peers.clone()).await;
    });
    loop {}
}

async fn print_available_peers(peers: Arc<Mutex<HashMap<IpAddr, StdinMsg>>>) {
    let mut interval = tokio::time::interval(Duration::from_millis(1500));
    loop {
        interval.tick().await;
        // only print this info when not in connection
        if IN_CONNECTION.load(Ordering::Relaxed) {
            continue;
        }
        println!("Peers Available:");
        {
            let set = peers.lock().unwrap();
            for (idx, elem) in set.iter().enumerate() {
                print!("  {}: {:?}: ", idx, elem.0);
                use std::io::Write;
                std::io::stdout().write_all(elem.1).unwrap();
                println!();
            }
        }
    }
}

// receives from STDIN and tries to create connections
async fn tcp_connection_creator(
    mut stdin_watch_receiver: tokio::sync::watch::Receiver<StdinMsg>,
    peers: Arc<Mutex<HashMap<IpAddr, StdinMsg>>>,
) {
    loop {
        if IN_CONNECTION.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        }
        stdin_watch_receiver.changed().await.unwrap();
        let num = {
            let inp = &*stdin_watch_receiver.borrow_and_update();
            if IN_CONNECTION.load(Ordering::Relaxed) {
                continue;
            }
            // turn u8's into utf8
            let len = inp
                .iter()
                .position(|&item| item == b'\n')
                .unwrap_or(inp.len());
            let inp = &inp[..len];
            let inp = std::str::from_utf8(inp).unwrap_or("ERRENOUS UTF8");
            println!("you just said: {:?}", inp);
            let Ok(num) = inp.parse::<u16>() else {
                println!("ERROR: {:?} is not a number", inp);
                continue;
            };
            num
        };
        let peer = {
            let set = peers.lock().unwrap();

            let Some(peer) = set.iter().skip(num as _).next() else {
                println!("ERROR: no such peer exists!");
                continue;
            };
            *peer.clone().0
        };
        let Ok(tcp_stream) = tokio::net::TcpStream::connect((peer, PORT)).await else {
            println!("ERROR: Could not connect!");
            continue;
        };
        spawn_connection_from(tcp_stream, stdin_watch_receiver.clone()).await;
    }
}

async fn tcp_connection_listener(
    stdin_watch_receiver: tokio::sync::watch::Receiver<StdinMsg>,
) -> Result<(), Box<dyn std::error::Error>> {
    let tcp = TcpListener::bind(LISTEN_SOCKET).await?;
    loop {
        let (stream, addr) = tcp.accept().await?;
        println!("got connection from: {:?}", addr);

        spawn_connection_from(stream, stdin_watch_receiver.clone()).await;
    }
}

async fn spawn_connection_from(
    stream: tokio::net::TcpStream,
    stdin_watch_receiver: tokio::sync::watch::Receiver<StdinMsg>,
) {
    if IN_CONNECTION.load(Ordering::Relaxed) {
        println!("BUT already in connection");
        return;
    }
    IN_CONNECTION.store(true, Ordering::Relaxed);
    println!("CONNECTION CREATED!");
    let (read_half, write_half) = stream.into_split();
    let atom = Arc::new(std::sync::atomic::AtomicBool::new(false));
    tokio::spawn(tcp_connection_receiver(read_half, atom.clone()));
    tokio::spawn(tcp_connection_writer(
        write_half,
        stdin_watch_receiver.clone(),
        atom
    ));
}

async fn tcp_connection_writer(
    mut write: tokio::net::tcp::OwnedWriteHalf,
    mut stdin_watch_receiver: tokio::sync::watch::Receiver<StdinMsg>,
    done: Arc<std::sync::atomic::AtomicBool>,
) {
    loop {
        stdin_watch_receiver.changed().await.unwrap();
        let mut my_buf = [0; 32];
        stdin_watch_receiver
            .borrow_and_update()
            .clone_into(&mut my_buf);
        if &my_buf[0..4] == b"exit" || &my_buf[0..4] == b"quit" {
            println!("MANUALLY QUITING CONNECTION!");
            write.shutdown().await.unwrap();
            break;
        }
        if write.write_all(&my_buf).await.is_err() || done.load(Ordering::Relaxed)  {
            break;
        }
    }
    if !done.load(Ordering::Relaxed) {
        done.store(true, Ordering::Relaxed);
        IN_CONNECTION.store(false, Ordering::Relaxed);
    }
    println!("exit tcp sender");
}

/// read from TcpListener and print to STDOUT
async fn tcp_connection_receiver(mut read: tokio::net::tcp::OwnedReadHalf, done: Arc<std::sync::atomic::AtomicBool>) {
    let mut stdout = tokio::io::stdout();
    loop {
        let mut buf = [0; 256];
        let Ok(len) = read.read(&mut buf).await else {
            println!("exit tcplistener");
            IN_CONNECTION.store(false, Ordering::Relaxed);
            break;
        };
        if len == 0 || done.load(Ordering::Relaxed) {
            break;
        }
        stdout.write_all(&buf).await.unwrap();
    }
    if !done.load(Ordering::Relaxed) {
        done.store(true, Ordering::Relaxed);
        IN_CONNECTION.store(false, Ordering::Relaxed);
    }
    println!("exit tcplistener");
}

async fn stdin_listener(
    watch: tokio::sync::watch::Sender<StdinMsg>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Listening on STDIN");

    let mut stdin = tokio::io::stdin();

    loop {
        let mut buf = [0; 32];
        stdin.read(&mut buf).await?;
        // println!("you wrote: {:?}", buf);
        watch.send(buf)?;
    }
}

async fn broadcast_existance(name: StdinMsg) {
    let udp_sock = tokio::net::UdpSocket::bind(("0.0.0.0", 0)).await.unwrap();
    udp_sock.set_broadcast(true).unwrap();
    println!("broadcasting existance");
    loop {
        udp_sock
            .send_to(&name, ("255.255.255.255", PORT as _))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(EXISTANCE_BROADCAST_MILLIS)).await;
    }
}
async fn listen_for_peers(peers: Arc<Mutex<HashMap<IpAddr, StdinMsg>>>, name: StdinMsg) {
    let udp_sock = tokio::net::UdpSocket::bind(("0.0.0.0", PORT as _))
        .await
        .unwrap();
    let mut buf = [0; 32];
    println!("listening for peers");
    loop {
        let (_size, peer) = udp_sock.recv_from(&mut buf).await.unwrap();
        if name == buf {
            // they have the same name as us
            continue;
        }

        if {
            let mut set = peers.lock().unwrap();
            set.insert(peer.ip(), buf).is_none()
        } {
            // added a new person
            print!("got new peer: {:?}, name:", peer);
            tokio::io::stdout().write_all(&buf).await.unwrap();
            tokio::io::stdout().write_all(b"\n").await.unwrap();
        }
    }
}
