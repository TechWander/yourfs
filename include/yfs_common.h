#ifndef YFS_COMMON_H
#define YFS_COMMON_H  

#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <ctype.h>
#include "./yfs_define.h"

void write_log(const char *fuc)
{
  time_t timep;
  time (&timep);
  FILE *fp = fopen("/tmp/a.txt","a");
  fprintf(fp,"%s %s\n",fuc,asctime( gmtime(&timep) ));
  fclose(fp);
}



char *rtrim(char *str) {
    char *end = str + strlen(str) - 1;
    while (end >= str && isspace(*end)) {
        end--;
    }
    *(end + 1) = '\0';
    return str;
}



void delete_prefix(char *str,  int n)
{
    int len = strlen(str);
    if (n >= len) {
        str[0] = '\0';
    } else {
        memmove(str, str + n, len - n + 1);
    }
}



void get_str_array(char *buff,char **array,int *count)
{
    char *token;
    *count = 0;

    delete_prefix(buff,  1);
    token = strtok(buff, "/");
    while (token != NULL)
    {
        strcpy(array[*count],token);
        *count = *count +1;
        token = strtok(NULL, "/");
    }

}


// allocate memory
char **ArrMalloc(int rows, int cols)
{

  char **data = (char **)malloc(rows*sizeof(char *));
  for (int i = 0; i < rows; i++)
  {
        data[i] = (char *)malloc(cols*sizeof(char));
   }
  return data;
}



// realloc
char **ArrRealloc(char **data,int rows_increase_number, int rows, int cols)
{
   int new_size = rows+rows_increase_number;
   data = (char **)realloc(data, new_size * sizeof(char *));
   for (int i = rows; i < new_size; i++)
   {
        data[i] = (char *)malloc(cols*sizeof(char));
   }
   return data;

}



void ArrFree(char **data,int rows, int cols)
{
  for (int i = 0; i < rows; i++)
    {
          free(data[i]);
    }
    free(data);
}


//get parant and name
void get_path(char *path, char *parent, char *name)
{
  char **array = ArrMalloc(MAX_FOLDER_DEEP, MAX_NAME_LENGTH);
  char buff[MAX_PATH_BUFF];
  int count=0;
  if(strcmp(path,"/")!=0) {
     strcpy(buff,path);
     get_str_array(buff,array,&count);
     strcpy(name,array[count-1]);

     int name_len = strlen(array[count-1])+1;
     int path_len = strlen(path);

     if(name_len == path_len) {
         strncpy(parent,path,1);
         parent[1]='\0';
     } else {
        strncpy(parent,path,path_len-name_len);
	parent[path_len-name_len]='\0';
     }
  }
  ArrFree(array,MAX_FOLDER_DEEP, MAX_NAME_LENGTH);
}



#endif



