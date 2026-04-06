# Quick Start Guide

Get the chat application running in 60 seconds!

## Step 1: Build the Application

```bash
cd /Users/taaliman/projects/luxquant/axiom/examples/apps/chat
verum build
```

## Step 2: Start the Server

Open a terminal and run:

```bash
verum run -- --server
```

You should see:
```
╔════════════════════════════════════════╗
║         Verum Chat v1.0.0              ║
║  Demonstrating Async & Networking      ║
╚════════════════════════════════════════╝

Starting server on port 8080...
Press Ctrl+C to stop the server.

[2025-12-06 10:00:00] [INFO] Starting chat server on port 8080
[2025-12-06 10:00:00] [INFO] Server listening on 0.0.0.0:8080
```

## Step 3: Connect First Client

Open another terminal:

```bash
verum run -- --client localhost 8080 alice
```

You should see:
```
╔════════════════════════════════════════╗
║         Verum Chat v1.0.0              ║
║  Demonstrating Async & Networking      ║
╚════════════════════════════════════════╝

Connecting to localhost:8080 as alice...

Connected to server!
Joined chat! Type /help for commands.

[10:00:05] *** Welcome to the chat, alice! Type /help for commands. ***
[10:00:05] Users online: 1 user(s) online: alice
>
```

## Step 4: Connect Second Client

Open a third terminal:

```bash
verum run -- --client localhost 8080 bob
```

Now Alice's terminal shows:
```
[10:00:10] *** bob joined the chat ***
```

And Bob sees:
```
Connected to server!
Joined chat! Type /help for commands.

[10:00:10] *** Welcome to the chat, bob! Type /help for commands. ***
[10:00:10] Users online: 2 user(s) online: alice, bob
>
```

## Step 5: Send Messages

**In Alice's terminal:**
```
> Hello bob!
[10:00:15] <alice> Hello bob!
```

**In Bob's terminal:**
```
[10:00:15] <alice> Hello bob!
> Hi alice!
[10:00:20] <bob> Hi alice!
```

**In Alice's terminal:**
```
[10:00:20] <bob> Hi alice!
```

## Step 6: Try Commands

**List users:**
```
> /list
[10:00:25] Users online: 2 user(s) online: alice, bob
```

**Send private message:**
```
> /w bob This is a secret message
[10:00:30] Private message sent to bob
```

Bob receives:
```
[10:00:30] [PM from alice] This is a secret message
```

**Get help:**
```
> /help
Available commands:
/help, /h        - Show this help
/quit, /q, /exit - Exit the chat
/list, /users    - List online users
/w <user> <msg>  - Send private message

Just type a message and press Enter to send to all users.
```

**Exit the chat:**
```
> /quit
Disconnecting from server...
Goodbye!
```

Alice's terminal shows:
```
[10:00:35] *** bob left the chat ***
```

## Common Issues

### Port Already in Use
If you see `Failed to bind to port 8080: address already in use`, use a different port:

```bash
# Server
verum run -- --server 9000

# Client
verum run -- --client localhost 9000 alice
```

### Invalid Username
Usernames must be 2-20 characters, alphanumeric plus `_` and `-`:

```bash
# Valid
verum run -- --client localhost 8080 alice
verum run -- --client localhost 8080 bob_2024
verum run -- --client localhost 8080 user-123

# Invalid
verum run -- --client localhost 8080 a           # Too short
verum run -- --client localhost 8080 "user name" # Spaces not allowed
verum run -- --client localhost 8080 user@host   # @ not allowed
```

### Connection Refused
Make sure the server is running before connecting clients!

## Testing with Multiple Clients

Open multiple terminals and connect different users:

```bash
# Terminal 1 - Server
verum run -- --server

# Terminal 2
verum run -- --client localhost 8080 alice

# Terminal 3
verum run -- --client localhost 8080 bob

# Terminal 4
verum run -- --client localhost 8080 charlie

# Terminal 5
verum run -- --client localhost 8080 diana
```

Now you have a 4-person chat room! Try:
- Broadcasting messages to everyone
- Sending private messages between specific users
- Listing all online users
- Having users join and leave

## Next Steps

- Read the full [README.md](README.md) for detailed documentation
- Explore the source code to see Verum language features:
  - `src/protocol.vr` - Refinement types and message protocol
  - `src/server.vr` - Async server with context system
  - `src/client.vr` - Interactive client with dual async loops
  - `src/main.vr` - CLI parsing and application entry point
- Modify the code to add new features!
- Run the test suite: `verum test`
- Run benchmarks: `verum bench`

## Tips

1. **Use Ctrl+C** to stop the server cleanly
2. **Use /quit** to exit clients gracefully
3. **Username tab completion** would be a great feature to add!
4. **Message history** could be stored in a database
5. **Rooms/Channels** could allow topic-based conversations
6. **Color coding** is implemented but can be enhanced

Enjoy exploring Verum's async/await, context system, and type safety features!
