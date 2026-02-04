#ifndef YFS_PASSWD_H
#define YFS_PASSWD_H

#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
#include <sys/types.h>
#include <fuse.h>
#include <sys/types.h>
#include <nfsc/libnfs.h>
#include "./yfs_define.h"
#include "./yourfs.h"
#include "./yfs_common.h"

#if defined(POSTGRES)

#if defined(UBUNTU)
#include <postgresql/libpq-fe.h>
#endif

#if defined(ROCKY)
#include <libpq-fe.h>
#endif

#endif


static bool yfs_postgres_check_key(char *ip, char *port, char *dbname, char *table, char *username, char *passwd, char *fs_key, char *user_path) {
     
     PGconn *conn;
     PGresult *res;
     char sql[4096];
     int num_rows=0,num_cols=0;

     conn =  PQsetdbLogin(ip,port,NULL,NULL,dbname,username,passwd);
     if (PQstatus(conn) == CONNECTION_BAD) {
         fprintf(stderr,"Connection to database '%s' failed!\n",dbname);
         PQfinish(conn);
         return false;
     }
     
     sprintf(sql,"SELECT user_type,user_email FROM %s WHERE fs_key='%s' LIMIT 1",table,fs_key);
     res = PQexec(conn, sql);

     num_rows = PQntuples(res);
     num_cols = PQnfields(res);

     if(num_rows == 0) {
        PQclear(res);
        PQfinish(conn);
        return false;
      }

     // key exit
      if(strcmp(rtrim(PQgetvalue(res, 0, 0)),"admin") == 0) {
           PQclear(res);
           PQfinish(conn);
           return true;
      } else {
           char user_email[256];
           FILE *fptr;
           if ((fptr = fopen(user_path, "r")) == NULL) {
                 printf("Error! please init system firstly.\n");
                 PQclear(res);
                 PQfinish(conn);
                 return false;
           }

           int status = fscanf(fptr,"%s", user_email);
           fclose(fptr);

           if(strcmp(rtrim(PQgetvalue(res, 0, 1)),user_email) == 0) {
               PQclear(res);
               PQfinish(conn);
               return true;
           } else {
               PQclear(res);
               PQfinish(conn);
               return false;
           }
      }
}


static void yfs_check_pass(char *fs_key){
   // here you can define
   char fs_tablename[128]="yfs_key",fs_dbname[128]="test",fs_username[128]="test",fs_passwd[128]="test";
   char *fs_ip=NULL, *fs_port = NULL;

    if((fs_ip = getenv("FS_IP")) && (fs_port = getenv("FS_PORT"))) {
    } else {
       fprintf(stderr,"Get database info failed!\n");
       exit(1);
    }

   bool key_pass = yfs_postgres_check_key(fs_ip, fs_port, fs_dbname, fs_tablename, fs_username, fs_passwd, fs_key, "/usr/uinfo/user");
   if(!key_pass) {
       fprintf(stderr,"Wrong passwd!\n");
       exit(1);
   }

}


#endif



