# yourfs

#### Introduce
Yourfs is a read-only file system with file isolation capabilities, implementing the necessary POSIX protocols. It allows users to mount remote NFS shares transparently through interfaces on their local systems, providing a simple and flexible way to access file system resources on the network, supporting V3 and V4 protocols, and defining file access scopes through a database. This open-source project greatly simplifies the need for cross-network file access, supports mainstream Linux systems, and makes cross-platform data sharing easy.

#### Software Architecture
The basic interface of the file system is defined through the VFS operation interface, and the database and file system interfaces are bound to achieve file-level permission isolation. Data is remotely loaded and operated through the NFS protocol during data reading.

#### Installation Tutorial

1.  Ubuntu/Debian

    `apt-get update`

    `apt-get install libpq-dev libsqlite3-dev libnfs-dev libfuse-dev fuse libtool m4 automake make`

    `dpkg -i ubutu_version_.deb`


2.  Rocky/Centos
     
    `yum update`

    `yum install epel-release`

    `yum install libpq-devel（using postgresql-devel to replace for centos7） libsq3-devel libnfs-devel fuse-devel fuse libtool m4 automake make`

    `rpm -ivh rocky_version_.rpm  --replacefiles`


#### Usage
Building this file system mainly involves database construction and mounting operations. The database records the files that the file system can access, and after mounting, the system will display accessible data based on the templates in the database tables. To facilitate easy use, this software provides the yourfs_tool command for building the database (currently supports sqlite3, postgres).

1.  Generate the database


    `yourfs_tool -s /path -o data.txt  # scan the file structure under the /path directory of the local system and write it to the data.txt file`

    `yourfs_tool -F sqlite3 -M10 -i data.txt -X data.db -Z data  # write the contents of the data.txt file into the data.db database, with the table name being data (before writing, you can modify the content of the data.txt file to achieve the needs of isolating files or modifying file path locations)`

2.  Mount the folder

    `yourfs -n nfs://127.0.0.1/path -m ./tmp/ -F sqlite3 -X data.db -Z data -M 10 -A # Asynchronously mount the mountable path /path to the ./tmp path`

3.  Unmount

    `umount ./tmp`

