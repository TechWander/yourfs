/*
   Copyright (C) by Jun Han <hanjun@nao.cas.cn> 2023
*/

#define FUSE_USE_VERSION 26
#define _FILE_OFFSET_BITS 64

#include <fuse.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <getopt.h>
#include <poll.h>
#include <pthread.h>
#include <unistd.h>
#include <sys/types.h>
#include <nfsc/libnfs.h>
#include  "../include/yourfs.h"
#include "../include/yfs_define.h"
#include "../include/yfs_key.h"

#if defined(SQLITE3)
#include "../include/yfs_sqlite3.h"
#endif

#if defined(POSTGRES)
#include "../include/yfs_postgres.h"
#endif

#if defined(PASSWD)
#include "../include/yfs_passwd.h"
#endif

static pthread_mutex_t nfs_mutex = PTHREAD_MUTEX_INITIALIZER;

struct nfs_context *nfs = NULL;

int custom_uid = -1;
int custom_gid = -1;

uid_t mount_user_uid;
gid_t mount_user_gid;

int yourfs_allow_other_own_ids=0;
int fuse_default_permissions=1;
int fuse_multithreads=1;


static int map_uid(int possible_uid) {
    if (custom_uid != -1 && possible_uid == custom_uid){
        return fuse_get_context()->uid;
    }
    return possible_uid;
}


static int map_gid(int possible_gid) {
    if (custom_gid != -1 && possible_gid == custom_gid){
        return fuse_get_context()->gid;
    }
    return possible_gid;
}


struct sync_cb_data {
	int is_finished;
	int status;

	void *return_data;
	size_t max_size;
};

	
	
static void wait_for_nfs_reply(struct nfs_context *nfs, struct sync_cb_data *cb_data) {
	struct pollfd pfd;
	int revents;
	int ret;
	static pthread_mutex_t reply_mutex = PTHREAD_MUTEX_INITIALIZER;

	pthread_mutex_lock(&reply_mutex);
	while (!cb_data->is_finished) {
	  pfd.fd = nfs_get_fd(nfs);
	  pfd.events = nfs_which_events(nfs);
	  pfd.revents = 0;

	  ret = poll(&pfd, 1, 100);
	  if (ret < 0)  {
	     revents = -1;
	  }
	  else {
	    revents = pfd.revents;
	  }

	  pthread_mutex_lock(&nfs_mutex);
	  ret = nfs_service(nfs, revents);
	  pthread_mutex_unlock(&nfs_mutex);
	  if (ret < 0) {
	    cb_data->status = -EIO;
	    break;
	  }
	}
	pthread_mutex_unlock(&reply_mutex);
}


static void generic_cb(int status, struct nfs_context *nfs, void *data, void *private_data) {
	struct sync_cb_data *cb_data = private_data;
	cb_data->is_finished = 1;
	cb_data->status = status;
}


static void update_rpc_credentials(void) {
        if (custom_uid == -1  && !yourfs_allow_other_own_ids) {
		nfs_set_uid(nfs, fuse_get_context()->uid);
	} 
        else if ((custom_uid == -1 ||  fuse_get_context()->uid != mount_user_uid) && yourfs_allow_other_own_ids) {
		nfs_set_uid(nfs, fuse_get_context()->uid);
	} 
	else {
		nfs_set_uid(nfs, custom_uid);
	}

	if (custom_gid == -1 && !yourfs_allow_other_own_ids) {
		nfs_set_gid(nfs, fuse_get_context()->gid);
        }
       	else if ((custom_gid == -1 || fuse_get_context()->gid != mount_user_gid) && yourfs_allow_other_own_ids) {
		nfs_set_gid(nfs, fuse_get_context()->gid);
	}
       	else {
		nfs_set_gid(nfs, custom_gid);
	}
}

static void stat64_cb(int status, struct nfs_context *nfs, void *data, void *private_data) {
	struct sync_cb_data *cb_data = private_data;

	cb_data->is_finished = 1;
	cb_data->status = status;

	if (status < 0) { return; }
	memcpy(cb_data->return_data, data, sizeof(struct nfs_stat_64));
}


static int yourfs_getattr(const char *path, struct FUSE_STAT *stbuf) {

     struct yourfs_data *userdata = (struct yourfs_data *) fuse_get_context()->private_data;
     struct nfs_stat_64 st;
     struct sync_cb_data cb_data;
     int ret;
     char realpath[1024]="/";


     memset(&cb_data, 0, sizeof(struct sync_cb_data));
     cb_data.return_data = &st;

    
     if(strcmp(path,"/")!=0) {
        if(!userdata->exists(path,realpath,userdata)) {
            pthread_mutex_unlock(&nfs_mutex);
	    return -ENOENT;
          }
      }

     pthread_mutex_lock(&nfs_mutex);
     update_rpc_credentials();
     ret = nfs_lstat64_async(nfs, realpath, stat64_cb, &cb_data);
     pthread_mutex_unlock(&nfs_mutex);

     if (ret < 0) { return ret; }
     wait_for_nfs_reply(nfs, &cb_data);

     stbuf->st_dev          = st.nfs_dev;
     stbuf->st_ino          = st.nfs_ino;
     stbuf->st_mode         = st.nfs_mode;
     stbuf->st_nlink        = st.nfs_nlink;
     stbuf->st_uid          = map_uid(st.nfs_uid);
     stbuf->st_gid          = map_gid(st.nfs_gid);
     stbuf->st_rdev         = st.nfs_rdev;
     stbuf->st_size         = st.nfs_size;
     stbuf->st_blksize      = st.nfs_blksize;
     stbuf->st_blocks       = st.nfs_blocks;

#if defined(HAVE_ST_ATIM)
     stbuf->st_atim.tv_sec  = st.nfs_atime;
     stbuf->st_atim.tv_nsec = st.nfs_atime_nsec;
     stbuf->st_mtim.tv_sec  = st.nfs_mtime;
     stbuf->st_mtim.tv_nsec = st.nfs_mtime_nsec;
     stbuf->st_ctim.tv_sec  = st.nfs_ctime;
     stbuf->st_ctim.tv_nsec = st.nfs_ctime_nsec;
#else
     stbuf->st_atime      = st.nfs_atime;
     stbuf->st_mtime      = st.nfs_mtime;
     stbuf->st_ctime      = st.nfs_ctime;
     stbuf->st_atime_nsec = st.nfs_atime_nsec;
     stbuf->st_mtime_nsec = st.nfs_mtime_nsec;
     stbuf->st_ctime_nsec = st.nfs_ctime_nsec;
#endif
     return cb_data.status;

}



static int yourfs_readdir(const char *path, void *buf, fuse_fill_dir_t filler, off_t offset, struct fuse_file_info *fi) {
    
    pthread_mutex_lock(&nfs_mutex); 
    struct yourfs_data *userdata = (struct yourfs_data *) fuse_get_context()->private_data;
    userdata->readdir(path, buf,userdata, filler);
    pthread_mutex_unlock(&nfs_mutex);
    return 0;

}


static void readlink_cb(int status, struct nfs_context *nfs, void *data, void *private_data) {
	
	struct sync_cb_data *cb_data = private_data;

	cb_data->is_finished = 1;
	cb_data->status = status;
	if (status < 0)  {  return;  }
	strncat(cb_data->return_data, data, cb_data->max_size);
}

static int yourfs_readlink(const char *path, char *buf, size_t size) {
	
	struct sync_cb_data cb_data;
	int ret;

        memset(&cb_data, 0, sizeof(struct sync_cb_data));
	*buf = 0;
	cb_data.return_data = buf;
	cb_data.max_size = size;

	pthread_mutex_lock(&nfs_mutex);
        update_rpc_credentials();
	ret = nfs_readlink_async(nfs, path, readlink_cb, &cb_data);
	pthread_mutex_unlock(&nfs_mutex);
	if (ret < 0) {  return ret; }
	wait_for_nfs_reply(nfs, &cb_data);

	return cb_data.status;
}

static void open_cb(int status, struct nfs_context *nfs, void *data, void *private_data) {
	
	struct sync_cb_data *cb_data = private_data;

	cb_data->is_finished = 1;
	cb_data->status = status;

	if (status < 0) { return; }
	cb_data->return_data = data;
}

static int yourfs_open(const char *path, struct fuse_file_info *fi) {

	struct yourfs_data *userdata = (struct yourfs_data *) fuse_get_context()->private_data;
	char realpath[MAX_PATH_BUFF];
        if(!userdata->exists(path,realpath,userdata)) {  return -ENOENT; }

	struct sync_cb_data cb_data;
	int ret; 

        memset(&cb_data, 0, sizeof(struct sync_cb_data));

	pthread_mutex_lock(&nfs_mutex);
        update_rpc_credentials();

	fi->fh = 0;
        ret = nfs_open_async(nfs, realpath, fi->flags, open_cb, &cb_data);
	pthread_mutex_unlock(&nfs_mutex);
	if (ret < 0) { return ret; }
	wait_for_nfs_reply(nfs, &cb_data);

	fi->fh = (uint64_t)cb_data.return_data;
        
	return cb_data.status;
}

static int yourfs_release(const char *path, struct fuse_file_info *fi) {

	struct sync_cb_data cb_data;
	struct nfsfh *nfsfh = (struct nfsfh *)fi->fh;

        memset(&cb_data, 0, sizeof(struct sync_cb_data));

	pthread_mutex_lock(&nfs_mutex);
	nfs_close_async(nfs, nfsfh, generic_cb, &cb_data);
	pthread_mutex_unlock(&nfs_mutex);
	wait_for_nfs_reply(nfs, &cb_data);

	return 0;
}

static void read_cb(int status, struct nfs_context *nfs, void *data, void *private_data) {

	struct sync_cb_data *cb_data = private_data;

	cb_data->is_finished = 1;
	cb_data->status = status;

	if (status < 0) { return; }
	memcpy(cb_data->return_data, data, status);
}

static int yourfs_read(const char *path, char *buf, size_t size, off_t offset, struct fuse_file_info *fi) {

	struct nfsfh *nfsfh = (struct nfsfh *)fi->fh;
	struct sync_cb_data cb_data;
	int ret;

        memset(&cb_data, 0, sizeof(struct sync_cb_data));
	cb_data.return_data = buf;

	pthread_mutex_lock(&nfs_mutex);
	update_rpc_credentials();
	ret = nfs_pread_async(nfs, nfsfh, offset, size, read_cb, &cb_data);
	pthread_mutex_unlock(&nfs_mutex);
	if (ret < 0) { return ret; }
	wait_for_nfs_reply(nfs, &cb_data);

	return cb_data.status;
}


static void statvfs_cb(int status, struct nfs_context *nfs, void *data, void *private_data) {
	
	struct sync_cb_data *cb_data = private_data;

	cb_data->is_finished = 1;
	cb_data->status = status;

	if (status < 0) { return; }
	memcpy(cb_data->return_data, data, sizeof(struct statvfs));
}

static int yourfs_statfs(const char *path, struct statvfs* stbuf) {
        
	int ret;
        struct statvfs svfs;

	struct sync_cb_data cb_data;

        memset(&cb_data, 0, sizeof(struct sync_cb_data));
	cb_data.return_data = &svfs;

	pthread_mutex_lock(&nfs_mutex);
        ret = nfs_statvfs_async(nfs, path, statvfs_cb, &cb_data);
	pthread_mutex_unlock(&nfs_mutex);
	if (ret < 0) { return ret; }
	wait_for_nfs_reply(nfs, &cb_data);
  
        stbuf->f_bsize      = svfs.f_bsize;
        stbuf->f_frsize     = svfs.f_frsize;
        stbuf->f_fsid       = svfs.f_fsid;
        stbuf->f_flag       = svfs.f_flag;
        stbuf->f_namemax    = svfs.f_namemax;
        stbuf->f_blocks     = svfs.f_blocks;
        stbuf->f_bfree      = svfs.f_bfree;
        stbuf->f_bavail     = svfs.f_bavail;
        stbuf->f_files      = svfs.f_files;
        stbuf->f_ffree      = svfs.f_ffree;
        stbuf->f_favail     = svfs.f_favail;

	return cb_data.status;
}


static struct fuse_operations nfs_oper = {
	.getattr	= yourfs_getattr,
	.open		= yourfs_open,
	.read		= yourfs_read,
	.readdir	= yourfs_readdir,
	.readlink	= yourfs_readlink,
	.release	= yourfs_release,
        .statfs 	= yourfs_statfs,
};



void print_usage(char *name)
{
	printf("Yourfs Usage(v1.0) : %s\n",name);
	printf( "\t [-?|--help] \n"
			"\nyourfs options : \n"
		//	"\t [-U NFS_UID|--yourfs_uid=NFS_UID] \n"
		//	"\t\t The uid passed within the rpc credentials within the mount point \n"
		//	"\t\t This is the same as passing the uid within the url, however if both are defined then the url's one is used\n"
		//	"\t [-G NFS_GID|--yourfs_gid=NFS_GID] \n"
		//		"\t\t The gid passed within the rpc credentials within the mount point \n"
		//	"\t\t This is the same as passing the gid within the url, however if both are defined then the url's one is used\n"
		//	"\t [-o|--yourfs_allow_other_own_ids] \n"
		//	"\t\t Allow yourfs with allow_user activated to update the rpc credentials with the current (other) user credentials instead\n"
		//		"\t\t of using the mount user credentials or (if defined) the custom credentials defined with -U/-G / url \n" 
		//	"\t\t This option activate allow_other, note that allow_other need user_allow_other to be defined in fuse.conf \n"
		//	"\nlibnfs options : \n"
			"\t [-n share_point| The server export to be mounted] \n"
		//	"\t\t The server export to be mounted \n"
			"\t [-m mount_path|  The client mount path] \n"
		//	"\t\t The client mount point \n"
		//	"\nfuse options (see man mount.fuse): \n"
		//	"\t [-p [0|1]|--default_permissions=[0|1]] \n"
		//	"\t\t The fuse default_permissions option do not have any argument , for compatibility with previous yourfs version default is activated (1)\n"
		//	"\t\t with the possibility to overwrite this behavior (0) \n"
		//	"\t [-t [0|1]| Multi-threaded by default (1)] \n"
		//	"\t\t Multi-threaded by default (1) \n"
			"\t [-a|--allow_other] \n"
		//	"\t [-r|--allow_root] \n"
		//	"\t [-u FUSE_UID|--uid=FUSE_UID] \n"
		//	"\t [-g FUSE_GID|--gid=FUSE_GID] \n"
		//	"\t [-K UMASK|--umask=UMASK] \n"
			"\t [-d|--direct_io] \n"
			"\t [-k|--kernel_cache] \n"
			"\t [-c|--auto_cache] \n"
		//	"\t [-E TIMEOUT|--entry_timeout=TIMEOUT] \n"
		//	"\t [-N TIMEOUT|--negative_timeout=TIMEOUT] \n"
		//	"\t [-T TIMEOUT|--attr_timeout=TIMEOUT] \n"
		//	"\t [-C TIMEOUT|--ac_attr_timeout=TIMEOUT] \n"
		//	"\t [-L|--logfile=logfile] \n"
		//	"\t [-l|--large_read] \n"
		//	"\t [-R MAX_READ|--max_read=MAX_READ] \n"
		//	"\t [-H MAX_READAHEAD|--max_readahead=MAX_READAHEAD] \n"
			"\t [-A|--async_read] \n"
			"\t [-S|--sync_read] \n"
		//	"\t [-h|--hard_remove] \n"
			"\t [-Y|--nonempty | setup it, if mount path is not empty] \n"
		//	"\t [-q|--use_ino] \n"
		//	"\t [-Q|--readdir_ino] \n"
		//	"\t [-f FSNAME| Default is the SHARE provided with -m] \n"
		//	"\t\t Default is the SHARE provided with -m \n"
		//	"\t [-s SUBTYPE|--subtype=SUBTYPE] \n"
		//	"\t\t Default is yourfs with kernel prefexing with fuse. \n"
		//	"\t [-b|--blkdev] \n"
		//	"\t [-D|--debug] \n"
		//	"\t [-i|--intr] \n"
		//	"\t [-I SIGNAL|--intr_signal=SIGNAL] \n"
		//	"\t [-O|--read_only] \n"
			"\ndatabase options : \n"
			"\t [-F database_type ]  It is used to filter database type to read.(you should compile sqlite3,postgres module to support)\n"
			"\t [-X database ] \n"

                        #if defined(SQLITE3)                        
			"\t\tsqlite3:  /path/data.db\n"
                        #endif

                        #if defined(POSTGRES)
                        "\t\tpostgres: ip,port,user,passwd,dbname\n"
                        #endif

                        "\t [-Z tablename| the table name to read] \n"

                        #if defined(SQLITE3)
    	 		"\t [-M MB| default 10, only used to setup sqlite cache] \n"
                        #endif
                        
                        #if defined(PASSWD)
			"\t [-P passwd ] \n"
                        #endif
                       "\n\t (If you have any question or bug report, please connect hanjun@nao.cas.cn) \n"
			);
	exit(0);
}

int main(int argc, char *argv[])
{
	mount_user_uid=getuid();
	mount_user_gid=getgid();

	int ret = 0;
	static struct option long_opts[] = {
		/*yourfs options*/
		{ "help", no_argument, 0, '?' },
		{ "nfs_share", required_argument, 0, 'n' },
		{ "mountpoint", required_argument, 0, 'm' },
		{ "yourfs_uid", required_argument, 0, 'U' },
		{ "yourfs_gid", required_argument, 0, 'G' },
		{ "yourfs_allow_other_own_ids", no_argument, 0, 'o' },
		/*fuse options*/
		{ "allow_other", no_argument, 0, 'a' },
		{ "uid", required_argument, 0, 'u' },
		{ "gid", required_argument, 0, 'g' },
		{ "debug", no_argument, 0, 'D' },
		{ "default_permissions", required_argument, 0, 'p' },
		{ "direct_io", no_argument, 0, 'd' },
		{ "allow_root", no_argument, 0, 'r' },
		{ "kernel_cache", no_argument, 0, 'k' },
		{ "auto_cache", no_argument, 0, 'c' },
		{ "large_read", no_argument, 0, 'l' },
                { "logfile", required_argument, 0, 'L' },
		{ "hard_remove", no_argument, 0, 'h' },
		{ "fsname", required_argument, 0, 'f' },
		{ "subtype", required_argument, 0, 's' },
		{ "blkdev", no_argument, 0, 'b' },
		{ "intr", no_argument, 0, 'i' },
		{ "max_read", required_argument, 0, 'R' },
		{ "max_readahead", required_argument, 0, 'H' },
		{ "async_read", no_argument, 0, 'A' },
		{ "sync_read", no_argument, 0, 'S' },
		{ "umask", required_argument, 0, 'K' },
		{ "entry_timeout", required_argument, 0, 'E' },
		{ "negative_timeout", required_argument, 0, 'N' },
		{ "attr_timeout", required_argument, 0, 'T' },
		{ "ac_attr_timeout", required_argument, 0, 'C' },
		{ "nonempty", no_argument, 0, 'Y' },
		{ "intr_signal", required_argument, 0, 'I' },
		{ "use_ino", no_argument, 0, 'q' },
		{ "readdir_ino", required_argument, 0, 'Q' },
		{ "multithread", required_argument, 0, 't' },
		{ "read_only", no_argument, 0, 'O' },
		// database
		{"database",required_argument,0,'X'},
		{"table",required_argument,0,'Z'},
		{"cache_memory",required_argument,0,'M'},
		{"filter_database",required_argument,0,'F'},
		{"passwd",required_argument,0,'P'},
		{"yourkey",required_argument,0,'y'},
		{ NULL, 0, 0, 0 }
	};

	int c;
	int opt_idx = 0;
	char *url = NULL;
	char *mnt = NULL;
	char *idstr = NULL;

	char fuse_uid_arg[32] = {0};
	char fuse_gid_arg[32] = {0};
	char fuse_fsname_arg[1024] = {0};
	char fuse_subtype_arg[1024] = {0};
	char fuse_max_write_arg[32] = {0};
	char fuse_max_read_arg[32] = {0};
	char fuse_max_readahead_arg[32] = {0};
	char fuse_Umask_arg[32] = {0};
	char fuse_entry_timeout_arg[32] = {0};
	char fuse_negative_timeout_arg[32] = {0};
	char fuse_attr_timeout_arg[32] = {0};
	char fuse_ac_attr_timeout_arg[32] = {0};
	char fuse_intr_signal_arg[32] = {0};

	struct nfs_url *urls = NULL;

	struct yourfs_data userdata;
	int    cache_memory = 10000;
	char   database_type[32];
	char   ip[32],port[10],username[32],passwd[32],dbname[32],fs_key[128]="hanjun@nao.cas.cn",your_key[10]="FADEP8";

        int yourfs_argc = 2;	
	char *yourfs_argv[37] = {
		"yourfs",
		"<export>",
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
		NULL,
        };

	while ((c = getopt_long(argc, argv, "?am:n:U:G:u:g:Dp:drklL:hf:s:biR:H:ASK:E:N:T:C:oYI:qQct:OX:Z:M:F:P:y:", long_opts, &opt_idx)) > 0) {
		switch (c) {
		case '?':
			print_usage(argv[0]);
			return 0;
		case 'a':
			yourfs_argv[yourfs_argc++] = "-oallow_other";
			break;
		case 'm':
			mnt = strdup(optarg);
			yourfs_argv[yourfs_argc++] = "-oro";            // used to open readonly
                      //  yourfs_argv[yourfs_argc++] = "-oallow_other";   // used to allow other 
			break;
		case 'n':
			url = strdup(optarg);
			break;
		case 'U':
			custom_uid=atoi(optarg);
			break;
		case 'G':
			custom_gid=atoi(optarg);
			break;
		case 'u':
			snprintf(fuse_uid_arg, sizeof(fuse_uid_arg), "-ouid=%s", optarg);
			yourfs_argv[yourfs_argc++] = fuse_uid_arg;
			break;
		case 'g':
			snprintf(fuse_gid_arg, sizeof(fuse_gid_arg), "-ogid=%s", optarg);
			yourfs_argv[yourfs_argc++] = fuse_gid_arg;
			break;
		case 'D':
			yourfs_argv[yourfs_argc++] = "-odebug";
			break;
		case 'p':
			fuse_default_permissions=atoi(optarg);
			break;
		case 't':
			fuse_multithreads=atoi(optarg);
			break;
		case 'd':
			yourfs_argv[yourfs_argc++] = "-odirect_io";
			break;
		case 'r':
			yourfs_argv[yourfs_argc++] = "-oallow_root";
			break;
		case 'k':
			yourfs_argv[yourfs_argc++] = "-okernel_cache";
			break;
		case 'c':
			yourfs_argv[yourfs_argc++] = "-oauto_cache";
			break;
		case 'l':
			yourfs_argv[yourfs_argc++] = "-olarge_read";
			break;
                case 'L':
                        logfile = strdup(optarg);
                        break;
		case 'h':
			yourfs_argv[yourfs_argc++] = "-ohard_remove";
			break;
		case 'f':
			snprintf(fuse_fsname_arg, sizeof(fuse_fsname_arg), "-ofsname=%s", optarg);
			yourfs_argv[yourfs_argc++] = fuse_fsname_arg;
			break;
		case 's':
			snprintf(fuse_subtype_arg, sizeof(fuse_subtype_arg), "-osubtype=%s", optarg);
			yourfs_argv[yourfs_argc++] = fuse_subtype_arg;
			break;
		case 'b':
			yourfs_argv[yourfs_argc++] = "-oblkdev";
			break;
		case 'i':
			yourfs_argv[yourfs_argc++] = "-ointr";
			break;
		case 'R':
			snprintf(fuse_max_read_arg, sizeof(fuse_max_read_arg), "-omax_read=%s", optarg);
			yourfs_argv[yourfs_argc++] = fuse_max_read_arg;
			break;
		case 'H':
			snprintf(fuse_max_readahead_arg, sizeof(fuse_max_readahead_arg), "-omax_readahead=%s", optarg);
			yourfs_argv[yourfs_argc++] = fuse_max_readahead_arg;
			break;
		case 'A':
			yourfs_argv[yourfs_argc++] = "-oasync_read";
			break;
		case 'S':
			yourfs_argv[yourfs_argc++] = "-osync_read";
			break;
		case 'K':
			snprintf(fuse_Umask_arg, sizeof(fuse_Umask_arg), "-oumask=%s", optarg);
			yourfs_argv[yourfs_argc++] = fuse_Umask_arg;
			break;
		case 'E':
			snprintf(fuse_entry_timeout_arg, sizeof(fuse_entry_timeout_arg), "-oentry_timeout=%s", optarg);
			yourfs_argv[yourfs_argc++] = fuse_entry_timeout_arg;
			break;
		case 'N':
			snprintf(fuse_negative_timeout_arg, sizeof(fuse_negative_timeout_arg), "-onegative_timeout=%s", optarg);
			yourfs_argv[yourfs_argc++] = fuse_negative_timeout_arg;
			break;
		case 'T':
			snprintf(fuse_attr_timeout_arg, sizeof(fuse_attr_timeout_arg), "-oattr_timeout=%s", optarg);
			yourfs_argv[yourfs_argc++] = fuse_attr_timeout_arg;
			break;
		case 'C':
			snprintf(fuse_ac_attr_timeout_arg, sizeof(fuse_ac_attr_timeout_arg), "-oac_attr_timeout=%s", optarg);
			yourfs_argv[yourfs_argc++] = fuse_ac_attr_timeout_arg;
			break;
		case 'o':
                        yourfs_allow_other_own_ids=1;
			break;
		case 'Y':
			yourfs_argv[yourfs_argc++] = "-ononempty";
			break;
		case 'I':
			snprintf(fuse_intr_signal_arg, sizeof(fuse_intr_signal_arg), "-ointr_signal=%s", optarg);
			yourfs_argv[yourfs_argc++] = fuse_intr_signal_arg;
			break;
		case 'q':
			yourfs_argv[yourfs_argc++] = "-ouse_ino";
			break;
		case 'Q':
			yourfs_argv[yourfs_argc++] = "-oreaddir_ino";
			break;
	        case 'O':
			yourfs_argv[yourfs_argc++] = "-oro";
			break;
		case 'X':
		        strcpy(userdata.database,optarg);
			break;
		case 'Z':
			strcpy(userdata.table,optarg);
			break;
                case 'M':
                        cache_memory = (int)(atof(optarg)*1000);
                        break;
		case 'F':
                        strcpy(database_type,optarg);
                        break;
		case 'P':
			strcpy(fs_key,optarg);
			break;
		case 'y':
		        strcpy(your_key,optarg);
			break;
		}
	}


	if (url == NULL) {
		fprintf(stderr, "-n was not specified.\n");
		print_usage(argv[0]);
		ret = 10;
		goto finished;
	}
	if (mnt == NULL) {
		fprintf(stderr, "-m was not specified.\n");
		print_usage(argv[0]);
		ret = 10;
		goto finished;
	}

		if(!key_is_good(your_key))
		  {
		      printf("    Please update yourfs key, connect hanjun@nao.cas.cn\n");
	      	      return 0;
		  }
	
	#if defined(PASSWD)
        yfs_check_pass(fs_key); 
        #endif

	
	/* Set allow_other if not defined and fusenfs_allow_other_own_ids defined */
	if (yourfs_allow_other_own_ids)
	{
		int i = 0, allow_other_set=0;
		for(i ; i < yourfs_argc; ++i)
		{
			if(!strcmp(yourfs_argv[i], "-oallow_other"))
			{
				allow_other_set=1;
				break;
			}
		}
		if (!allow_other_set){yourfs_argv[yourfs_argc++] = "-oallow_other";}
	}

	/* Set default fsname if not defined */
	if (!strstr(fuse_fsname_arg, "-ofsname="))
	{
		snprintf(fuse_fsname_arg, sizeof(fuse_fsname_arg), "-ofsname=%s", url);
		yourfs_argv[yourfs_argc++] = fuse_fsname_arg;
	}

	/* Set default subtype if not defined */
	if (!strstr(fuse_subtype_arg, "-osubtype="))
	{
		snprintf(fuse_subtype_arg, sizeof(fuse_subtype_arg), "-osubtype=%s", "fuse-nfs");
		yourfs_argv[yourfs_argc++] = fuse_subtype_arg;
	}

	/* Only for compatibility with previous version */
	if (fuse_default_permissions) { 
		yourfs_argv[yourfs_argc++] = "-odefault_permissions";
	}

	if (!fuse_multithreads) {
		yourfs_argv[yourfs_argc++] = "-s";
	}
       
	// setup database
	if(strcmp(database_type,"sqlite3")==0 || strcmp(database_type,"postgres")==0) {

        }else{
             fprintf(stderr, "Please use -F to setup the database type, support sqlite3, postgres\n");
             goto finished;
	}


	#if defined(SQLITE3)
        if(strcmp(database_type,"sqlite3")==0) {

	  sqlite3 *db;
          sqlite3_open(userdata.database,&db);
	  char *zErrMsg = 0, sql[32];

	  // set up cache size for sqlite3
	  sprintf(sql,"PRAGMA cache_size=%d;",cache_memory);
	  int rc = sqlite3_exec(db, sql, 0, 0, &zErrMsg);
	
          if( rc != SQLITE_OK ){
              LOG("SQL error: %s\n", zErrMsg);
              sqlite3_free(zErrMsg);
	      goto finished;
            }else{
	       LOG("Starting set cache_size=%dKB sucessfully.\n",cache_memory);
           }

	 userdata.db = db;
         userdata.readdir = &yfs_sqlite3_readdir;
	 userdata.exists  = &yfs_sqlite3_exists;
	} 
	#endif


        #if defined(POSTGRES)
        if(strcmp(database_type,"postgres")==0) {
	   sscanf(userdata.database, "%[^,],%[^,],%[^,],%[^,],%s",ip, port, username,passwd,dbname);
	   PGconn *conn;
           PGresult *res;           
	   conn =  PQsetdbLogin(ip,port,NULL,NULL,dbname,username,passwd);
	   if (PQstatus(conn) == CONNECTION_BAD) {
               LOG("Connection to database '%s' failed!\n",dbname);
               PQfinish(conn);
               exit(1);
              }
           userdata.conn = conn;
           userdata.res  = res;
	   userdata.readdir = &yfs_postgres_readdir;
           userdata.exists  = &yfs_postgres_exists;
	}
	#endif



	nfs = nfs_init_context();
	if (nfs == NULL) {
		fprintf(stderr, "Failed to init context\n");
		ret = 10;
		goto finished;
	}

	urls = nfs_parse_url_dir(nfs, url);
	if (urls == NULL) {
		fprintf(stderr, "Failed to parse url : %s\n", nfs_get_error(nfs));
		ret = 10;
		goto finished;
	}

	if (idstr = strstr(url, "uid=")) { custom_uid = atoi(&idstr[4]); }
	if (idstr = strstr(url, "gid=")) { custom_gid = atoi(&idstr[4]); }

	ret = nfs_mount(nfs, urls->server, urls->path);
	if (ret != 0) {
		fprintf(stderr, "Failed to mount nfs share : %s\n", nfs_get_error(nfs));
		goto finished;
	}

	yourfs_argv[1] = mnt;
        

	LOG("Starting fuse_main()\n");
	ret = fuse_main(yourfs_argc, yourfs_argv, &nfs_oper, &userdata);

finished:
	nfs_destroy_url(urls);
	if (nfs != NULL) {
		nfs_destroy_context(nfs);
	}
	
	#if defined(SQLITE3)
	if(strcmp(database_type,"sqlite3")==0) {
	   sqlite3_close(userdata.db);
	}
        #endif


	#if defined(POSTGRES)
	if(strcmp(database_type,"postgres")==0) {
	    PQfinish(userdata.conn);
	 }		 
        #endif


	free(url);
	free(mnt);
	return ret;
}
