#ifndef YFS_DEFINE_H
#define YFS_DEFINE_H  


#ifndef FUSE_STAT
#define FUSE_STAT stat
#endif


#define MAX_FOLDER_DEEP 128
#define MAX_NAME_LENGTH 256
#define MAX_PATH_BUFF   4096
#define MAX_SQL_LENGTH  20480

static char *logfile;

#define LOG(...) do {                                           \
        if (logfile) {                                          \
                FILE *fh = fopen(logfile, "a+");                \
                time_t t = time(NULL);                          \
                char tmp[256];                                  \
                strftime(tmp, sizeof(tmp), "%T", localtime(&t));\
                fprintf(fh, "[NFS] %s ", tmp);                  \
                fprintf(fh, __VA_ARGS__);                       \
                fclose(fh);                                     \
        }                                                       \
} while (0);


#endif



