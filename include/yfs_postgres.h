#ifndef YFS_POSTGRES_H
#define YFS_POSTGRES_H

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

static void  yfs_postgres_readdir(const char *path,void *buf, struct yourfs_data *userdata, fuse_fill_dir_t filler) {
  
  char sql[MAX_SQL_LENGTH];
  int num_rows=0,num_cols=0;

  sprintf(sql,"SELECT name FROM %s WHERE  parent='%s' order by name",userdata->table,path);
  userdata->res = PQexec(userdata->conn, sql);
  
  num_rows = PQntuples(userdata->res);
  num_cols = PQnfields(userdata->res);

  filler(buf, ".", NULL, 0);
  filler(buf, "..", NULL, 0);

  for (int i = 0; i < num_rows; i++) {
    for (int j = 0; j < num_cols; j++) {
	 filler(buf, PQgetvalue(userdata->res, i, j), NULL, 0);
      }
  }
  PQclear(userdata->res); 

}


static bool yfs_postgres_exists(const char *path,char *realpath, struct yourfs_data *userdata) {
   
   char sql[MAX_SQL_LENGTH];  

   if(strcmp(path,"/")==0) {
     memcpy(realpath,path,strlen(path)+1);
     return true;
   }

   char parent[MAX_PATH_BUFF],name[MAX_NAME_LENGTH],local_path[MAX_PATH_BUFF];
   int num_rows=0,num_cols=0;

   strcpy(local_path,path);
   get_path(local_path, parent, name);
   sprintf(sql,"SELECT realpath FROM %s WHERE parent='%s' and name='%s' LIMIT 1",userdata->table,parent,name);
   userdata->res = PQexec(userdata->conn, sql);
   
   num_rows = PQntuples(userdata->res);
   num_cols = PQnfields(userdata->res);

   if(num_rows == 0) {
       return false;
   }
   
   memcpy(realpath,PQgetvalue(userdata->res, 0, 0),strlen(PQgetvalue(userdata->res, 0, 0))+1);
   PQclear(userdata->res);
   
   return true;
}



void yfs_postgres_create_table(char *ip, char *port, char *username,char *passwd,char *dbname,char *table) {
    
     PGresult *res;	
     PGconn *conn;
     char sql[MAX_SQL_LENGTH];

     conn =  PQsetdbLogin(ip,port,NULL,NULL,dbname,username,passwd);
     if (PQstatus(conn) == CONNECTION_BAD) {
         fprintf(stderr,"Connection to database '%s' failed!\n",dbname);
         PQfinish(conn);
         exit(1);
     }
     
    sprintf(sql,"DROP TABLE IF EXISTS %s; \
		 CREATE TABLE %s (id SERIAL primary key, parent VARCHAR(4096), name VARCHAR(256), realpath VARCHAR(4096));",table,table);
    res = PQexec(conn, sql);
    if (PQresultStatus(res) != PGRES_COMMAND_OK) {
        fprintf(stderr, "Query failed: %s", PQerrorMessage(conn));
        PQclear(res);
        PQfinish(conn);
        exit(1);
    }
    printf("Table %s has been created.\n", table);
        
    PQclear(res);
    PQfinish(conn);

}


void yfs_postgres_write_table(char *ip, char *port, char *username, char *passwd, char *dbname, char *table, char *input_filename) {

     PGresult *res;
     PGconn *conn;
     char sql[MAX_SQL_LENGTH],name[MAX_NAME_LENGTH],parent[MAX_PATH_BUFF],realpath[MAX_PATH_BUFF],buffer[MAX_SQL_LENGTH];
     int id_num = 1;

     FILE *fp = fopen(input_filename,"r");
     conn =  PQsetdbLogin(ip,port,NULL,NULL,dbname,username,passwd);
     if (PQstatus(conn) == CONNECTION_BAD) {
         fprintf(stderr,"Connection to database '%s' failed!\n",dbname);
         PQclear(res);
         PQfinish(conn);
         exit(1);
     }
    
    sprintf(sql,"COPY %s(parent, name, realpath) FROM STDIN WITH DELIMITER ' ';",table);
    PQexec(conn, "BEGIN");
    PQexec(conn,sql);

    while(fscanf(fp,"%s  %s  %s",parent,name,realpath)!=EOF) {
        sprintf(buffer,"%s %s %s\n",parent,name,realpath);
        PQputCopyData(conn, buffer, strlen(buffer));
    }

    PQputCopyEnd(conn, NULL);
    PQexec(conn, "COMMIT");

    //
    sprintf(sql,"CREATE INDEX index_parent ON %s USING hash(parent); \
                 CREATE INDEX index_parent_name ON %s(parent,name);",table,table);
    PQexec(conn, sql);

    fclose(fp);
    PQclear(res);
    PQfinish(conn);
}

	
#endif



