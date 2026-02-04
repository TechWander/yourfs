#ifndef YFS_SQLITE3_H
#define YFS_SQLITE3_H

#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sqlite3.h>
#include <fuse.h>
#include <sys/types.h>
#include <nfsc/libnfs.h>
#include "./yfs_define.h"
#include "./yourfs.h"
#include "./yfs_common.h"


static void  yfs_sqlite3_readdir(const char *path,void *buf, struct yourfs_data *userdata, fuse_fill_dir_t filler) {

  char **azresult = NULL;
  int  n_row,n_col;
  char *zErrMsg = 0;
  char sql[MAX_SQL_LENGTH];
 
  sprintf(sql,"SELECT name FROM %s  INDEXED BY index_parent  WHERE  parent='%s' order by name",userdata->table,path);
  sqlite3_get_table(userdata->db,sql,&azresult,&n_row,&n_col,&zErrMsg);

  filler(buf, ".", NULL, 0);
  filler(buf, "..", NULL, 0);
  for(int i=1;i<(n_row+1)*n_col;i++) { 
        filler(buf, azresult[i], NULL, 0);
     }
  
  sqlite3_free_table(azresult);

}


static bool yfs_sqlite3_exists(const char *path,char *realpath, struct yourfs_data *userdata) {
   
   char sql[MAX_SQL_LENGTH];
   char **azresult = NULL;
   char *zErrMsg = 0;
   int  n_row,n_col;

   if(strcmp(path,"/")==0) { 
     memcpy(realpath,path,strlen(path)+1);
     return true; 
   }

   char parent[MAX_PATH_BUFF],name[MAX_NAME_LENGTH],local_path[MAX_PATH_BUFF];
   
   strcpy(local_path,path);
   get_path(local_path, parent, name);
   sprintf(sql,"SELECT realpath FROM %s INDEXED BY index_parent_name WHERE parent='%s' and name='%s' LIMIT 1",userdata->table,parent,name);
   sqlite3_get_table(userdata->db,sql,&azresult,&n_row,&n_col,&zErrMsg);
   
   if(n_row == 0) { 
       return false; 
   }

   memcpy(realpath,azresult[1],strlen(azresult[1])+1);
   sqlite3_free_table(azresult);
  
   return true;

}


void yfs_sqlite3_create_table(char *database, char *table_name) {
  
  sqlite3 *db;
  char *zErrMsg = 0;

  char sql[MAX_SQL_LENGTH];
  sprintf(sql,"DROP TABLE IF EXISTS %s; CREATE TABLE %s (id INTEGER PRIMARY KEY AUTOINCREMENT,parent CHAR(4096),name CHAR(256),realpath CHAR(4096)); CREATE INDEX index_id ON %s(id); CREATE INDEX index_parent ON %s(parent); CREATE INDEX index_parent_name on %s(parent, name);",table_name,table_name,table_name,table_name,table_name);
  sqlite3_open(database,&db);
  sqlite3_exec(db, sql, 0, 0, &zErrMsg);
  sqlite3_close(db);

}



void yfs_sqlite3_write_table(char *database, char *table_name, char *input_filename, int cache_memory) {

  sqlite3 *db;
  sqlite3_stmt *pPrepare;
  char *zErrMsg = 0,sql[MAX_SQL_LENGTH],name[MAX_NAME_LENGTH],parent[MAX_PATH_BUFF],realpath[MAX_PATH_BUFF],cache_setup[MAX_NAME_LENGTH];
  int id_num = 1;
  FILE *fp=fopen(input_filename,"r");
  sqlite3_open(database,&db);
  sqlite3_exec(db, "PRAGMA synchronous = OFF;", 0, 0, 0);
  sprintf(cache_setup,"PRAGMA cache_size=%d;",cache_memory);
  sqlite3_exec(db, cache_setup, 0, 0, 0);
  sprintf(sql,"INSERT INTO %s(id,parent,name,realpath) VALUES (?,?,?,?);",table_name);
  sqlite3_prepare_v2(db, sql, -1, &pPrepare, NULL);
  sqlite3_exec(db, "BEGIN", 0, 0, 0);
  while(fscanf(fp,"%s  %s  %s",parent,name,realpath)!=EOF) {
    
    sqlite3_reset(pPrepare);
    sqlite3_bind_int(pPrepare, 1, id_num);
    sqlite3_bind_text(pPrepare, 2, parent,-1,0);
    sqlite3_bind_text(pPrepare, 3, name,-1,0);
    sqlite3_bind_text(pPrepare, 4, realpath, -1, 0);
    sqlite3_step(pPrepare);
    id_num++;

  }
  sqlite3_exec(db, "COMMIT", 0, 0, &zErrMsg);
  sqlite3_finalize(pPrepare);
  sqlite3_close(db);
  fclose(fp);
}


	
#endif



