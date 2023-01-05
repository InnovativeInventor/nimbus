## The Nimbus Filesystem
Nimbus is a virtual, networked filesystem that provides *upfront* safety guarantees to a user, intended for personal use.
In particular, it provides a filesystem that will FAIL LOUDLY and prompt the user if the consistency of reads and writes cannot be guaranteed.
The user is thereby empowered to prevent conflicts before they occur.

Nimbus is implemented with FUSE and is currently a work-in-progress.

## Modes of operation
Nimbus has two modes:
- development mode
- backup mode

#### Development mode
In development mode, when a project is being read/written to, nimbus will attempt to acquire a lock on the project. 
Development mode provides strong safety guarantees to the user. 
Intuitively, development mode enforces the notion that a user can only develop on one project/computer at a given moment. [^1]
If a lock *cannot* be acquired it will FAIL LOUDLY and prompt the user for action (this should happen rarely).
If a lock *can* be acquired, the user will not notice anything and everything will appear as normal.
As nimbus will FAIL LOUDLY and prompt iff a lock *cannot* be acquired, the user is guaranteed that all successful read and writes are occuring on the most up-to-date version of the project and is consistent.

By default, projects will only be cached/stored on disk if:
- you are currently accessing the project
- the project has been recently accessed (and there is sufficient space to store the project)
- the project is pinned

#### Backup mode
In backup mode, eventual consistency is guaranteed.
All operations are read-only.
By default, if there is space, all projects will be cached/stored on disk.

## Architecture 
TODO: fill in here.

[^1]: Until cloning is invented, this seems like a reasonable limitation.
